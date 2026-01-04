#!/usr/bin/env bash
set -Eeuo pipefail

usage() {
  cat <<'USAGE' >&2
Gemini CLI headless wrapper (safe defaults).

Reads stdin and forwards it to `gemini` (one-shot). Use a short positional prompt
to instruct how stdin should be handled.

Usage:
  cat prompt.txt | gemini_headless.sh [options] -- "<positional prompt...>"

Options:
  --model <name>            Gemini model (default: gemini-3-pro-preview)
  -o, --output-format <f>   text|json|stream-json (default: text)
  --approval-mode <mode>    default|auto_edit|yolo (default: default)
  --sandbox | --no-sandbox  Run Gemini CLI in sandbox (default: off)
  --auth <type>             gemini-api-key|vertex-ai|oauth-personal (default: auto-detect)
  --web                     Allow `google_web_search`
  --allow <tool>            Add an allowed tool (repeatable)
  --allow-mcp <name>        Add an allowed MCP server name (repeatable)
  --profile-dir <dir>       Base dir for isolated profiles (default: <repo>/.state/gemini-headless)
  --use-user-home           Do not isolate HOME (discouraged)
  --unsafe                  Allow dangerous tools like run_shell_command
  -h, --help                Show this help

Examples:
  cat /tmp/review_prompt.txt | \
    bash Scripts/gemini_headless.sh \
      --web -- "Respond to stdin; no edits."
USAGE
}

model="gemini-3-pro-preview"
output_format="text"
approval_mode="default"
sandbox="0"
web="0"
auth_type=""
profile_dir=""
use_user_home="0"
unsafe="0"

declare -a allowed_tools=()
declare -a allowed_mcp_servers=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --model)
      model="${2:?missing --model value}"
      shift 2
      ;;
    -o|--output-format)
      output_format="${2:?missing --output-format value}"
      shift 2
      ;;
    --approval-mode)
      approval_mode="${2:?missing --approval-mode value}"
      shift 2
      ;;
    --sandbox)
      sandbox="1"
      shift
      ;;
    --no-sandbox)
      sandbox="0"
      shift
      ;;
    --web)
      web="1"
      shift
      ;;
    --auth)
      auth_type="${2:?missing --auth value}"
      shift 2
      ;;
    --allow)
      allowed_tools+=("${2:?missing --allow value}")
      shift 2
      ;;
    --allow-mcp)
      allowed_mcp_servers+=("${2:?missing --allow-mcp value}")
      shift 2
      ;;
    --profile-dir)
      profile_dir="${2:?missing --profile-dir value}"
      shift 2
      ;;
    --use-user-home)
      use_user_home="1"
      shift
      ;;
    --unsafe)
      unsafe="1"
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    --)
      shift
      break
      ;;
    *)
      break
      ;;
  esac
done

if [[ "$output_format" != "text" && "$output_format" != "json" && "$output_format" != "stream-json" ]]; then
  echo "ERROR: invalid --output-format: $output_format (expected text|json|stream-json)" >&2
  exit 2
fi

if [[ "$approval_mode" != "default" && "$approval_mode" != "auto_edit" && "$approval_mode" != "yolo" ]]; then
  echo "ERROR: invalid --approval-mode: $approval_mode (expected default|auto_edit|yolo)" >&2
  exit 2
fi

repo_root="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
if [[ -z "$profile_dir" ]]; then
  profile_dir="$repo_root/.state/gemini-headless"
fi

orig_home="${HOME:-}"

profile_home="$HOME"
if [[ "$use_user_home" != "1" ]]; then
  mode="host"
  if [[ "$sandbox" == "1" ]]; then
    mode="sandbox"
  fi
  profile_home="$profile_dir/profiles/$mode"
fi

settings_dir="$profile_home/.gemini"
settings_path="$settings_dir/settings.json"
mkdir -p "$settings_dir"

detect_default_auth_type() {
  if [[ -n "${auth_type:-}" ]]; then
    echo "$auth_type"
    return
  fi

  if [[ -n "${GEMINI_API_KEY:-}" ]]; then
    echo "gemini-api-key"
    return
  fi

  if [[ -n "${GOOGLE_API_KEY:-}" || (-n "${GOOGLE_CLOUD_PROJECT:-}" && -n "${GOOGLE_CLOUD_LOCATION:-}") ]]; then
    echo "vertex-ai"
    return
  fi

  if [[ -n "$orig_home" && -f "$orig_home/.gemini/oauth_creds.json" ]]; then
    echo "oauth-personal"
    return
  fi

  echo "gemini-api-key"
}

effective_auth_type="$(detect_default_auth_type)"

if [[ -n "$auth_type" && "$auth_type" != "$effective_auth_type" ]]; then
  effective_auth_type="$auth_type"
fi

if [[ ! -f "$settings_path" ]]; then
  cat > "$settings_path" <<JSON
{
  "ide": { "enabled": false },
  "security": { "auth": { "selectedType": "${effective_auth_type}" } },
  "tools": { "autoAccept": false }
}
JSON
fi

if [[ "$effective_auth_type" == "oauth-personal" && -n "$orig_home" ]]; then
  if [[ ! -f "$settings_dir/oauth_creds.json" && -f "$orig_home/.gemini/oauth_creds.json" ]]; then
    cp -a "$orig_home/.gemini/oauth_creds.json" "$settings_dir/oauth_creds.json"
    chmod 600 "$settings_dir/oauth_creds.json" || true
  fi
  if [[ ! -f "$settings_dir/google_accounts.json" && -f "$orig_home/.gemini/google_accounts.json" ]]; then
    cp -a "$orig_home/.gemini/google_accounts.json" "$settings_dir/google_accounts.json"
    chmod 600 "$settings_dir/google_accounts.json" || true
  fi
fi

if [[ ${#allowed_tools[@]} -eq 0 ]]; then
  allowed_tools=(list_directory glob read_file read_many_files search_file_content)
fi
if [[ "$web" == "1" ]]; then
  allowed_tools+=(google_web_search)
fi

if [[ "$unsafe" != "1" ]]; then
  for tool in "${allowed_tools[@]}"; do
    if [[ "$tool" == run_shell_command* ]]; then
      echo "ERROR: refusing to allow '$tool' without --unsafe" >&2
      exit 3
    fi
  done
fi

cmd=(gemini -m "$model" -o "$output_format" --approval-mode "$approval_mode")
if [[ "$sandbox" == "1" ]]; then
  cmd+=(--sandbox)
fi

for tool in "${allowed_tools[@]}"; do
  cmd+=(--allowed-tools "$tool")
done
for server in "${allowed_mcp_servers[@]}"; do
  cmd+=(--allowed-mcp-server-names "$server")
done

if [[ $# -eq 0 ]]; then
  echo "ERROR: missing positional prompt. Use: -- \"Respond to stdin; ...\"" >&2
  exit 2
fi

exec env HOME="$profile_home" "${cmd[@]}" "$@"

