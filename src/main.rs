use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use yansi::Paint;
use zellij_tile::prelude::*;

const BUILD_LABEL: &str = "zellij-worktree 0.1.0 (live-cwd fix)";

#[derive(Debug, Clone, PartialEq)]
enum Mode {
    List,
    Create,
    DeleteConfirm,
}

#[derive(Debug, Clone)]
struct WorktreeInfo {
    path: String,
    branch: Option<String>,
    is_current: bool,
}

struct State {
    mode: Mode,
    input: String,
    worktrees: Vec<WorktreeInfo>,
    selected_index: usize,
    error_message: Option<String>,
    cwd_diagnostic: Option<String>,
    waiting_for_command: bool,
    repo_root: Option<String>,
    working_directory: Option<PathBuf>,
    base_path: Option<String>,
    initialized: bool,
    first_render: bool,
    refresh_pending: bool,
    active_tab_position: Option<usize>,
    pane_manifest: Option<PaneManifest>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            mode: Mode::List,
            input: String::new(),
            worktrees: Vec::new(),
            selected_index: 0,
            error_message: None,
            cwd_diagnostic: None,
            waiting_for_command: false,
            repo_root: None,
            working_directory: None,
            base_path: None,
            initialized: false,
            first_render: true,
            refresh_pending: false,
            active_tab_position: None,
            pane_manifest: None,
        }
    }
}

register_plugin!(State);

impl State {
    fn parse_worktree_list(&mut self, output: &[u8]) {
        let output = String::from_utf8_lossy(output);
        // Use fully qualified syntax to avoid yansi's deprecated Paint::clear()
        Vec::clear(&mut self.worktrees);

        let mut current_path: Option<String> = None;

        for line in output.lines() {
            if let Some(new_current_path) = line.strip_prefix("worktree ") {
                current_path = Some(new_current_path.to_string());
            } else if let Some(path) = &current_path {
                if let Some(current_branch) = line.strip_prefix("branch ") {
                    let is_current = self.repo_root.as_ref().map(|p| p == path).unwrap_or(false);
                    self.worktrees.push(WorktreeInfo {
                        path: path.to_string(),
                        branch: Some(current_branch.to_string()),
                        is_current,
                    });
                    current_path = None;
                } else if line.starts_with("detached") {
                    let is_current = self.repo_root.as_ref().map(|p| p == path).unwrap_or(false);
                    self.worktrees.push(WorktreeInfo {
                        path: path.to_string(),
                        branch: None,
                        is_current,
                    });
                    current_path = None;
                }
            }
        }
        // Filter out the main worktree (usually first one)
        if !self.worktrees.is_empty() {
            self.worktrees.remove(0);
        }

        self.selected_index = 0;
    }

    fn resolve_worktree_path(&self, input: &str) -> Option<String> {
        // Absolute paths
        if input.starts_with('/') || input.starts_with('~') {
            return Some(input.to_string());
        }

        // Relative paths starting with ./ or ../
        if input.starts_with("./") || input.starts_with("../") {
            if let Some(repo_root) = &self.repo_root {
                let repo_path = Path::new(repo_root);
                return Some(repo_path.join(input).to_string_lossy().to_string());
            }
            return None;
        }

        // Branch names - create in base_path or parent directory
        if let Some(base_path) = &self.base_path {
            Some(format!("{}/{}", base_path, input))
        } else if let Some(repo_root) = &self.repo_root {
            let parent = Path::new(repo_root)
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| ".".to_string());
            Some(format!("{}/{}", parent, input))
        } else {
            None
        }
    }

    fn get_tab_name(&self, path: &str) -> String {
        Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("worktree")
            .to_string()
    }

    fn clear_state(&mut self) {
        // Use fully qualified syntax to avoid yansi's deprecated Paint::clear()
        String::clear(&mut self.input);
        self.error_message = None;
        self.cwd_diagnostic = None;
        self.waiting_for_command = false;
        self.selected_index = 0;
    }

    fn request_git_refresh(&mut self) {
        self.initialized = false;
        self.repo_root = None;
        self.working_directory = None;
        // Use fully qualified syntax to avoid yansi's deprecated Paint::clear()
        Vec::clear(&mut self.worktrees);
        self.error_message = None;
        self.waiting_for_command = true;
        self.refresh_pending = true;
        self.mode = Mode::List;
        // Use fully qualified syntax to avoid yansi's deprecated Paint::clear()
        String::clear(&mut self.input);

        self.try_start_git_refresh();
    }

    fn try_start_git_refresh(&mut self) {
        if !self.refresh_pending {
            return;
        }

        let Some(tab_position) = self.active_tab_position else {
            return;
        };
        let Some(pane_manifest) = &self.pane_manifest else {
            return;
        };
        let Some(focused_pane) = get_focused_pane(tab_position, pane_manifest) else {
            self.fail_refresh("No focused terminal pane found");
            return;
        };

        let pane_id = PaneId::Terminal(focused_pane.id);
        let pane_cwd = match get_pane_cwd(pane_id) {
            Ok(cwd) => cwd,
            Err(error) => {
                self.fail_refresh(format!(
                    "Could not determine focused pane working directory: {error}"
                ));
                return;
            }
        };

        self.working_directory = Some(pane_cwd.clone());
        self.refresh_pending = false;

        let mut context = BTreeMap::new();
        context.insert("command".to_string(), "rev-parse".to_string());
        let pane_pid = get_pane_pid(pane_id).ok().filter(|pid| *pid > 0);
        self.cwd_diagnostic = Some(format!(
            "pane={} pid={} zellij_cwd={}",
            focused_pane.id,
            pane_pid
                .map(|pid| pid.to_string())
                .unwrap_or_else(|| "unknown".to_string()),
            pane_cwd.display()
        ));
        if let Some(pane_pid) = pane_pid {
            context.insert("cwd_source".to_string(), "process-tree".to_string());
            Self::launch_repository_discovery(pane_pid, &pane_cwd, context);
        } else {
            Self::launch_git_command(&["rev-parse", "--show-toplevel"], &pane_cwd, context);
        }
    }

    fn fail_refresh(&mut self, message: impl Into<String>) {
        self.refresh_pending = false;
        self.waiting_for_command = false;
        self.error_message = Some(message.into());
    }

    fn launch_git_command(args: &[&str], cwd: &Path, context: BTreeMap<String, String>) {
        let mut command = Vec::with_capacity(args.len() + 1);
        command.push("git");
        command.extend_from_slice(args);
        run_command_with_env_variables_and_cwd(
            &command,
            BTreeMap::new(),
            cwd.to_path_buf(),
            context,
        );
    }

    fn launch_repository_discovery(
        pane_pid: i32,
        fallback_cwd: &Path,
        context: BTreeMap<String, String>,
    ) {
        const SCRIPT: &str = "pane_pid=$1; target_pid=$(ps -o tpgid= -p \"$pane_pid\" 2>/dev/null); set -- $target_pid; target_pid=${1:-$pane_pid}; [ \"$target_pid\" -gt 0 ] 2>/dev/null || target_pid=$pane_pid; exec git -C /proc/$target_pid/cwd rev-parse --show-toplevel";
        let pane_pid = pane_pid.to_string();
        run_command_with_env_variables_and_cwd(
            &["sh", "-c", SCRIPT, "zellij-worktree", &pane_pid],
            BTreeMap::new(),
            fallback_cwd.to_path_buf(),
            context,
        );
    }

    fn run_git_command(
        &self,
        args: &[&str],
        context: BTreeMap<String, String>,
    ) -> Result<(), String> {
        let repo_root = self
            .repo_root
            .as_deref()
            .ok_or_else(|| "Could not determine Git repository directory".to_string())?;
        Self::launch_git_command(args, Path::new(repo_root), context);
        Ok(())
    }
}

impl ZellijPlugin for State {
    fn load(&mut self, configuration: BTreeMap<String, String>) {
        request_permission(&[
            PermissionType::ReadApplicationState,
            PermissionType::ChangeApplicationState,
            PermissionType::RunCommands,
        ]);

        subscribe(&[
            EventType::Key,
            EventType::RunCommandResult,
            EventType::TabUpdate,
            EventType::PaneUpdate,
            EventType::Visible,
        ]);

        if let Some(base_path) = configuration.get("base_path") {
            self.base_path = Some(base_path.clone());
        }
    }

    fn update(&mut self, event: Event) -> bool {
        match event {
            Event::Key(key) => {
                if self.waiting_for_command {
                    return false;
                }

                match key.bare_key {
                    BareKey::Esc if self.mode != Mode::List => {
                        self.mode = Mode::List;
                        self.clear_state();
                    }
                    BareKey::Esc => {
                        close_self();
                    }
                    BareKey::Char('c') if key.has_modifiers(&[KeyModifier::Ctrl]) => {
                        close_self();
                    }
                    BareKey::Enter => match self.mode {
                        Mode::List => {
                            if let Some(worktree) = self.worktrees.get(self.selected_index) {
                                let tab_name = self.get_tab_name(&worktree.path);
                                new_tab(Some(&tab_name), Some(&worktree.path));
                                close_self();
                            }
                        }
                        Mode::Create => {
                            if !self.input.is_empty() {
                                if let Some(path) = self.resolve_worktree_path(&self.input) {
                                    let tab_name = self.get_tab_name(&path);
                                    self.waiting_for_command = true;
                                    self.error_message = None;

                                    let mut context = BTreeMap::new();
                                    context
                                        .insert("command".to_string(), "worktree-add".to_string());
                                    context.insert("tab_name".to_string(), tab_name);
                                    context.insert("path".to_string(), path.clone());
                                    if let Err(error) =
                                        self.run_git_command(&["worktree", "add", &path], context)
                                    {
                                        self.waiting_for_command = false;
                                        self.error_message = Some(error);
                                    }
                                } else {
                                    self.error_message = Some(
                                        [
                                            "Could not resolve path".to_string(),
                                            self.repo_root.clone().unwrap_or_default(),
                                        ]
                                        .concat(),
                                    );
                                }
                            }
                        }
                        Mode::DeleteConfirm => {
                            if let Some(worktree) = self.worktrees.get(self.selected_index) {
                                self.waiting_for_command = true;
                                self.error_message = None;

                                let mut context = BTreeMap::new();
                                context
                                    .insert("command".to_string(), "worktree-remove".to_string());
                                if let Err(error) = self.run_git_command(
                                    &["worktree", "remove", &worktree.path],
                                    context,
                                ) {
                                    self.waiting_for_command = false;
                                    self.error_message = Some(error);
                                }
                            }
                        }
                    },
                    BareKey::Backspace => {
                        if self.mode == Mode::Create {
                            self.input.pop();
                        }
                    }
                    BareKey::Char('n') if key.has_no_modifiers() && self.mode == Mode::List => {
                        self.mode = Mode::Create;
                        // Use fully qualified syntax to avoid yansi's deprecated Paint::clear()
                        String::clear(&mut self.input);
                        self.error_message = None;
                    }
                    BareKey::Char('d') if key.has_no_modifiers() && self.mode == Mode::List => {
                        if !self.worktrees.is_empty() && self.selected_index < self.worktrees.len()
                        {
                            self.mode = Mode::DeleteConfirm;
                        }
                    }
                    BareKey::Up | BareKey::Char('k')
                        if key.has_no_modifiers() && self.mode == Mode::List =>
                    {
                        if !self.worktrees.is_empty() {
                            if self.selected_index > 0 {
                                self.selected_index -= 1;
                            } else {
                                self.selected_index = self.worktrees.len() - 1;
                            }
                        }
                    }
                    BareKey::Down | BareKey::Char('j')
                        if key.has_no_modifiers() && self.mode == Mode::List =>
                    {
                        if !self.worktrees.is_empty() {
                            if self.selected_index < self.worktrees.len() - 1 {
                                self.selected_index += 1;
                            } else {
                                self.selected_index = 0;
                            }
                        }
                    }
                    BareKey::Char(c) if c.is_ascii() && key.has_no_modifiers() => {
                        if self.mode == Mode::Create {
                            self.input.push(c);
                        }
                    }
                    _ => {}
                }
                true
            }
            Event::RunCommandResult(exit_code, stdout, stderr, context) => {
                let command_type = context.get("command").map(|s| s.as_str()).unwrap_or("");

                match command_type {
                    "rev-parse" => {
                        if exit_code == Some(0) {
                            let output = String::from_utf8_lossy(&stdout);
                            let path = output.trim().to_string();
                            if !path.is_empty() {
                                self.repo_root = Some(path);
                                let mut context = BTreeMap::new();
                                context.insert("command".to_string(), "worktree-list".to_string());
                                if let Err(error) = self
                                    .run_git_command(&["worktree", "list", "--porcelain"], context)
                                {
                                    self.waiting_for_command = false;
                                    self.error_message = Some(error);
                                }
                            } else {
                                self.waiting_for_command = false;
                                self.error_message =
                                    Some("Could not determine git root".to_string());
                            }
                        } else if context.get("cwd_source").map(String::as_str)
                            == Some("process-tree")
                        {
                            let proc_error = String::from_utf8_lossy(&stderr);
                            if let Some(diagnostic) = &mut self.cwd_diagnostic {
                                diagnostic.push_str(&format!(
                                    " | process_tree_error={} ({:?})",
                                    proc_error.trim(),
                                    exit_code
                                ));
                            }
                            if let Some(cwd) = self.working_directory.as_deref() {
                                let mut fallback_context = BTreeMap::new();
                                fallback_context
                                    .insert("command".to_string(), "rev-parse".to_string());
                                fallback_context
                                    .insert("cwd_source".to_string(), "zellij".to_string());
                                Self::launch_git_command(
                                    &["rev-parse", "--show-toplevel"],
                                    cwd,
                                    fallback_context,
                                );
                            } else {
                                self.fail_refresh("Could not determine focused pane directory");
                            }
                        } else if exit_code.is_none() {
                            self.fail_refresh("Failed to launch Git");
                        } else {
                            let cwd = self
                                .working_directory
                                .as_deref()
                                .map(|path| path.display().to_string())
                                .unwrap_or_else(|| "the focused pane directory".to_string());
                            self.fail_refresh(format!(
                                "Focused pane directory is not inside a Git repository: {cwd}"
                            ));
                        }
                    }
                    "worktree-list" => {
                        self.waiting_for_command = false;
                        match exit_code {
                            Some(0) => {
                                self.parse_worktree_list(&stdout);
                                self.initialized = true;
                            }
                            Some(code) => {
                                let error = String::from_utf8_lossy(&stderr);
                                self.error_message =
                                    Some(format!("Error ({}): {}", code, error.trim()));
                            }
                            None => {
                                self.error_message = Some("Failed to launch Git".to_string());
                            }
                        }
                    }
                    "worktree-add" => {
                        self.waiting_for_command = false;

                        match exit_code {
                            Some(0) => {
                                if let (Some(tab_name), Some(path)) =
                                    (context.get("tab_name"), context.get("path"))
                                {
                                    new_tab(Some(&tab_name), Some(&path));
                                    close_self();
                                }
                            }
                            Some(code) => {
                                let error = String::from_utf8_lossy(&stderr);
                                self.error_message =
                                    Some(format!("Error ({}): {}", code, error.trim()));
                            }
                            None => {
                                self.error_message = Some("Failed to launch Git".to_string());
                            }
                        }
                    }
                    "worktree-remove" => {
                        self.waiting_for_command = false;

                        match exit_code {
                            Some(0) => {
                                self.mode = Mode::List;
                                self.clear_state();
                                let mut ctx = BTreeMap::new();
                                ctx.insert("command".to_string(), "worktree-list".to_string());
                                match self
                                    .run_git_command(&["worktree", "list", "--porcelain"], ctx)
                                {
                                    Ok(()) => self.waiting_for_command = true,
                                    Err(error) => self.error_message = Some(error),
                                }
                            }
                            Some(code) => {
                                let error = String::from_utf8_lossy(&stderr);
                                self.error_message =
                                    Some(format!("Error ({}): {}", code, error.trim()));
                            }
                            None => {
                                self.error_message = Some("Failed to launch Git".to_string());
                            }
                        }
                    }
                    _ => {
                        // Unknown command type
                        self.waiting_for_command = false;
                    }
                }
                true
            }
            Event::TabUpdate(tabs) => {
                self.active_tab_position =
                    tabs.iter().find(|tab| tab.active).map(|tab| tab.position);
                self.try_start_git_refresh();
                false
            }
            Event::PaneUpdate(pane_manifest) => {
                self.pane_manifest = Some(pane_manifest);
                self.try_start_git_refresh();
                false
            }
            Event::Visible(is_visible) => {
                if is_visible && !self.waiting_for_command {
                    // Refresh git info when plugin becomes visible
                    self.request_git_refresh();
                }
                true
            }
            _ => false,
        }
    }

    fn render(&mut self, _rows: usize, _cols: usize) {
        if self.first_render {
            self.first_render = false;
            self.request_git_refresh();
        }

        println!("{}", BUILD_LABEL.bright_black());
        if let Some(diagnostic) = &self.cwd_diagnostic {
            println!("{}", diagnostic.bright_black());
        }

        if !self.initialized {
            if let Some(error) = &self.error_message {
                println!("{}", error.red());
                println!();
                println!("{}", "Press Esc to close".bright_black());
            } else {
                println!("{}", "Loading...".yellow());
            }
            return;
        }

        match self.mode {
            Mode::List => {
                println!("{}", "Worktrees".cyan().bold());
                println!(
                    "{}",
                    "[j/k] navigate | [Enter] open | [n] new | [d] delete".bright_black()
                );
                println!();

                if self.worktrees.is_empty() {
                    println!("{}", "No worktrees found".bright_black());
                    println!();
                    println!("{}", "Press [n] to create a new worktree".bright_black());
                } else {
                    for (i, wt) in self.worktrees.iter().enumerate() {
                        let marker = if i == self.selected_index { ">" } else { " " };
                        let current = if wt.is_current {
                            format!(" {}", "(current)".yellow())
                        } else {
                            String::new()
                        };
                        let branch = wt.branch.as_deref().unwrap_or("detached");
                        let short_path = wt.path.split('/').next_back().unwrap_or(&wt.path);

                        println!("{} {} {} {}", marker, short_path, branch.cyan(), current);
                    }
                }

                if let Some(error) = &self.error_message {
                    println!();
                    println!("{}", error.red());
                }
            }
            Mode::Create => {
                println!("{}", "Create Worktree".cyan().bold());
                println!("{}", "[Esc] back to list".bright_black());
                println!();
                print!("Path/branch: {}", self.input);
                println!("{}", "_".blink());

                if let Some(error) = &self.error_message {
                    println!();
                    println!("{}", error.red());
                }

                if self.waiting_for_command {
                    println!();
                    println!("{}", "Creating worktree...".yellow());
                }
            }
            Mode::DeleteConfirm => {
                if let Some(wt) = self.worktrees.get(self.selected_index) {
                    println!("{}", "Confirm Delete".red().bold());
                    println!();
                    println!("Delete worktree: {}", wt.path.cyan());
                    if let Some(branch) = &wt.branch {
                        println!("Branch: {}", branch.yellow());
                    }
                    println!();
                    println!("{}", "[Enter] confirm | [Esc] cancel".bright_black());

                    if let Some(error) = &self.error_message {
                        println!();
                        println!("{}", error.red());
                    }

                    if self.waiting_for_command {
                        println!();
                        println!("{}", "Deleting...".yellow());
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_worktrees_and_marks_the_detected_repository() {
        let mut state = State {
            repo_root: Some("/projects/topic".to_string()),
            ..State::default()
        };

        state.parse_worktree_list(
            b"worktree /projects/main\nHEAD abc\nbranch refs/heads/main\n\
              \nworktree /projects/topic\nHEAD def\nbranch refs/heads/topic\n",
        );

        assert_eq!(state.worktrees.len(), 1);
        assert_eq!(state.worktrees[0].path, "/projects/topic");
        assert_eq!(
            state.worktrees[0].branch.as_deref(),
            Some("refs/heads/topic")
        );
        assert!(state.worktrees[0].is_current);
    }

    #[test]
    fn resolves_relative_worktree_paths_from_the_repository_root() {
        let state = State {
            repo_root: Some("/projects/main".to_string()),
            ..State::default()
        };

        assert_eq!(
            state.resolve_worktree_path("../topic").as_deref(),
            Some("/projects/main/../topic")
        );
    }
}
