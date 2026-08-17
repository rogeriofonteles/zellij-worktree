#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/setup-workbench.sh [WORKBENCH_COMMAND]

Install the local zellij-worktree plugin and configure Ctrl+k to open selected
worktrees with nvim-zellij-workbench. WORKBENCH_COMMAND defaults to the sibling
nvim-zellij-workbench checkout's bin/surface-workbench script.

The current login's HOME is used. ZELLIJ_CONFIG_DIR can override the default
configuration directory, and ZELLIJ_CONFIG_FILE can override config.kdl.
EOF
}

if [[ ${1:-} == "-h" || ${1:-} == "--help" ]]; then
  usage
  exit 0
fi

if (( $# > 1 )); then
  usage >&2
  exit 2
fi

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/.." && pwd)"
default_workbench="$repo_root/../nvim-zellij-workbench/bin/surface-workbench"
workbench_command="${1:-$default_workbench}"

if [[ ! -x $workbench_command ]]; then
  printf 'Workbench command is not executable: %s\n' "$workbench_command" >&2
  printf 'Pass its path explicitly: %s /path/to/surface-workbench\n' "$0" >&2
  exit 1
fi
workbench_command="$(cd "$(dirname "$workbench_command")" && pwd)/$(basename "$workbench_command")"

config_dir="${ZELLIJ_CONFIG_DIR:-${XDG_CONFIG_HOME:-$HOME/.config}/zellij}"
mkdir -p "$config_dir"
config_dir="$(cd "$config_dir" && pwd)"
config_file="${ZELLIJ_CONFIG_FILE:-$config_dir/config.kdl}"
mkdir -p "$(dirname "$config_file")"
config_file="$(cd "$(dirname "$config_file")" && pwd)/$(basename "$config_file")"
plugin_file="$config_dir/plugins/zellij-worktree.wasm"
plugin_artifact="$repo_root/target/wasm32-wasip1/release/zellij-worktree.wasm"

if [[ ! -f $plugin_artifact ]]; then
  printf 'Building zellij-worktree...\n'
  cargo build --release --manifest-path "$repo_root/Cargo.toml"
fi

mkdir -p "$(dirname "$plugin_file")"
if [[ ! -f $plugin_file ]] || ! cmp -s "$plugin_artifact" "$plugin_file"; then
  plugin_tmp="$(mktemp "${plugin_file}.tmp.XXXXXX")"
  trap 'rm -f "${plugin_tmp:-}" "${config_tmp:-}"' EXIT
  cp "$plugin_artifact" "$plugin_tmp"
  chmod 755 "$plugin_tmp"
  mv "$plugin_tmp" "$plugin_file"
fi

kdl_escape() {
  local value=$1
  value=${value//\\/\\\\}
  value=${value//\"/\\\"}
  printf '%s' "$value"
}

plugin_uri="file:$(kdl_escape "$plugin_file")"
workbench_value="$(kdl_escape "$workbench_command")"
binding=$(cat <<EOF
    shared_except "locked" "tab" {
        bind "Ctrl k" {
            LaunchOrFocusPlugin "$plugin_uri" {
                floating true
                workbench_command "$workbench_value"
            }
        }
    }
EOF
)

config_tmp="$(mktemp "${config_file}.tmp.XXXXXX")"
trap 'rm -f "${plugin_tmp:-}" "${config_tmp:-}"' EXIT

if [[ ! -s $config_file ]]; then
  {
    printf 'keybinds {\n'
    printf '%s\n' "$binding"
    printf '}\n'
  } >"$config_tmp"
elif grep -Eq 'LaunchOrFocusPlugin.*zellij-worktree(\.wasm)?' "$config_file"; then
  awk -v plugin_uri="$plugin_uri" -v workbench="$workbench_value" '
    function brace_count(text, character, copy) {
      copy = text
      return gsub(character, "", copy)
    }
    {
      line = $0
      opens = brace_count(line, "\\{")
      closes = brace_count(line, "\\}")

      if (!in_target && line ~ /LaunchOrFocusPlugin/ && line ~ /zellij-worktree(\.wasm)?/) {
        in_target = 1
        target_depth = depth + opens - closes
        indent = line
        sub(/[^ ].*$/, "", indent)
        invocation_indent = indent
        sub(/"[^"]*zellij-worktree\.wasm"/, "\"" plugin_uri "\"", line)
      } else if (in_target && line ~ /^[[:space:]]*workbench_command[[:space:]]+/) {
        line = invocation_indent "    workbench_command \"" workbench "\""
        configured = 1
      }

      next_depth = depth + opens - closes
      if (in_target && next_depth < target_depth && !configured) {
        print invocation_indent "    workbench_command \"" workbench "\""
        configured = 1
      }

      print line
      depth = next_depth
      if (in_target && depth < target_depth) {
        in_target = 0
      }
    }
  ' "$config_file" >"$config_tmp"
elif grep -Eq '^[[:space:]]*keybinds[[:space:]]*\{' "$config_file"; then
  awk -v binding="$binding" '
    function brace_count(text, character, copy) {
      copy = text
      return gsub(character, "", copy)
    }
    {
      opens = brace_count($0, "\\{")
      closes = brace_count($0, "\\}")
      if (!in_keybinds && $0 ~ /^[[:space:]]*keybinds[[:space:]]*\{/) {
        in_keybinds = 1
        keybinds_depth = depth + opens - closes
      }
      next_depth = depth + opens - closes
      if (in_keybinds && next_depth < keybinds_depth && !inserted) {
        print binding
        inserted = 1
      }
      print
      depth = next_depth
      if (in_keybinds && depth < keybinds_depth) {
        in_keybinds = 0
      }
    }
  ' "$config_file" >"$config_tmp"
else
  {
    cat "$config_file"
    printf '\nkeybinds {\n%s\n}\n' "$binding"
  } >"$config_tmp"
fi

if cmp -s "$config_file" "$config_tmp"; then
  rm -f "$config_tmp"
  config_tmp=""
  printf 'Already configured: %s\n' "$config_file"
  exit 0
fi

if [[ -f $config_file ]]; then
  backup_file="${config_file}.before-workbench-setup"
  if [[ ! -e $backup_file ]]; then
    cp -p "$config_file" "$backup_file"
  fi
  chmod --reference="$config_file" "$config_tmp"
else
  chmod 644 "$config_tmp"
fi
mv "$config_tmp" "$config_file"
config_tmp=""

printf 'Configured: %s\n' "$config_file"
printf 'Plugin:     %s\n' "$plugin_file"
printf 'Workbench:  %s\n' "$workbench_command"
