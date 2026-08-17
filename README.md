# zellij-worktree

A Zellij plugin for managing git worktrees.

![demo](demo.gif)

## Features

- **List worktrees**: View all worktrees on plugin open
- **Open in new tab**: Select a worktree and open it in a new tab
- **Create worktrees**: Create new worktrees and open them in new tabs
- **Delete worktrees**: Delete worktrees with confirmation
- **Smart path resolution**: Branch names create worktrees in parent directory; full paths are used as-is
- **SSH mode**: Manage worktrees on another machine while keeping Zellij local

## Configuration and Installation

Add to your `~/.config/zellij/config.kdl`:

```kdl
plugins {
    worktree location="https://github.com/sharph/zellij-worktree/releases/latest/download/zellij-worktree.wasm"
}
```

Add a keybinding:

```kdl
shared_except "locked" "tab" {
    bind "Ctrl w" {
        LaunchOrFocusPlugin "https://github.com/sharph/zellij-worktree/releases/latest/download/zellij-worktree.wasm" {
            floating true
        }
    }
}
```

### Optional: Custom base path

```kdl
plugins {
    worktree location="https://github.com/sharph/zellij-worktree/releases/latest/download/zellij-worktree.wasm" {
        base_path "~/projects"
    }
}
```

### SSH mode (local Zellij, remote worktrees)

SSH mode is for sessions where Zellij runs locally but a terminal pane is connected to another
machine. The plugin runs Git over SSH and opens each selected worktree in a new local Zellij tab
that contains its own SSH connection. Zellij does not need to be installed or running on the
remote machine.

Configure both `remote_host` and `remote_repo` on the plugin launch action:

```kdl
shared_except "locked" "tab" {
    bind "Ctrl w" {
        LaunchOrFocusPlugin "file:~/.config/zellij/plugins/zellij-worktree.wasm" {
            floating true
            remote_host "dev-vm"
            remote_repo "/home/rogerio/projects/my-project"
        }
    }
}
```

- `remote_host` is passed directly to `ssh`. It can be a hostname, `user@host`, or an alias from
  `~/.ssh/config`.
- `remote_repo` is an absolute path on the remote machine to any worktree in the repository.
- `base_path`, when set, is also interpreted on the remote machine in SSH mode.

With the configuration above, the plugin obtains the list with the equivalent of:

```bash
ssh dev-vm "git -C /home/rogerio/projects/my-project worktree list --porcelain"
```

Choosing a worktree such as `/home/rogerio/projects/my-project-feature` creates a new local
Zellij tab running the equivalent of:

```bash
ssh -t dev-vm 'cd -- /home/rogerio/projects/my-project-feature && exec "$SHELL" -l'
```

Create and delete operations also execute remotely. Relative create paths are resolved from the
remote repository root, and branch names use the remote repository's parent directory unless
`base_path` is configured.

#### SSH requirements and recommendations

The plugin's discovery, create, and delete commands run in the background and cannot display an
interactive password prompt. Configure public-key authentication (and unlock the key in an SSH
agent) before using SSH mode. Verify it with:

```bash
ssh dev-vm true
```

SSH connection multiplexing avoids establishing a new connection for every operation:

```sshconfig
Host dev-vm
    HostName dev-vm.example.com
    User rogerio
    ControlMaster auto
    ControlPersist 10m
    ControlPath ~/.ssh/control-%C
```

The remote login environment must provide `git` and a POSIX-compatible shell. Because an SSH
client does not expose the current directory of its interactive remote shell to local Zellij,
`remote_repo` is required; the plugin does not infer it from the focused SSH pane.

To keep both local and remote shortcuts, define two bindings with different plugin configuration
(for example, `Ctrl w` for local worktrees and `Ctrl Shift w` for the remote host).

### Build from source

```bash
git clone https://github.com/sharph/zellij-worktree
cd zellij-worktree
cargo build --release
mkdir -p ~/.config/zellij/plugins
cp target/wasm32-wasip1/release/zellij-worktree.wasm ~/.config/zellij/plugins/
```

Then update your config to use the local plugin:

```kdl
shared_except "locked" "tab" {
    bind "Ctrl w" {
        LaunchOrFocusPlugin "file:~/.config/zellij/plugins/zellij-worktree.wasm" {
            floating true
            workbench_command "/absolute/path/to/nvim-zellij-workbench/bin/surface-workbench"
        }
    }
}
```

For the local `Ctrl+k` workbench setup, the repository also includes an idempotent helper. It
builds and installs the plugin, then updates the current login's Zellij config while preserving
the rest of the file:

```bash
./scripts/setup-workbench.sh
```

By default it uses `../nvim-zellij-workbench/bin/surface-workbench`. Pass a different executable
path as the first argument when the workbench checkout lives elsewhere. Re-running the script is
safe, including from a new SSH login with a different home directory.

## Usage

### Open Worktree

1. Press your keybinding (e.g., `Ctrl+w`)
2. Use `j`/`k` or arrow keys to navigate the list
3. Press `Enter` to open the selected worktree with `nvim-zellij-workbench`

### Create Worktree

1. Open the plugin
2. Press `n` to create a new worktree
3. Type a branch name or full path
   - Branch name: creates worktree at `base_path/<branch-name>` (if configured) or `../<branch-name>`
   - Relative path (starting with `./` or `../`): relative to repo root
   - Full path (starting with `/` or `~`): uses exact path
4. Press `Enter` to create the worktree and open a new tab

### Delete Worktree

1. Open the plugin
2. Use `j`/`k` or arrow keys to navigate to the worktree
3. Press `d` to request deletion
4. Press `Enter` to confirm deletion
5. Press `Esc` to cancel

### Keybindings

| Key | Action |
|-----|--------|
| `Esc` | Close plugin / Cancel action |
| `Ctrl+c` | Close plugin |
| `Enter` | Open selected worktree / Confirm action |
| `j`/`k` or ↑/↓ | Navigate list |
| `n` | Create new worktree |
| `d` | Delete selected worktree |

## Requirements

- Zellij 0.44.3 or later
- Git
- SSH client, key-based authentication, and Git on the remote host when using SSH mode

## License

MIT
