#!/usr/bin/env bash
# Prints what is new on GitHub for Claude since the last check: issues and
# comments from the maintainer (anything without the <!-- claude --> marker)
# and failed CI runs on main. Prints nothing when there is nothing to do,
# so a periodic check costs one short command.
#
#   scripts/issue-inbox.sh          show what is new since the last check
#   scripts/issue-inbox.sh --done   record that everything shown is handled
#
# State lives outside the repo, in $HOME/.claude/yakvc-issues-state.json.
set -euo pipefail

if [ -z "${IN_NIX_SHELL:-}" ]; then
	exec nix develop "$(dirname "$0")/.." --command "$0" "$@"
fi

export GH_CONFIG_DIR="$HOME/.claude/gh"
repo=desocketed/yakvc
state="$HOME/.claude/yakvc-issues-state.json"
[ -f "$state" ] || echo '{"last_check": "1970-01-01T00:00:00Z", "handled": {}}' >"$state"

if [ "${1:-}" = "--done" ]; then
	now=$(date -u +%Y-%m-%dT%H:%M:%SZ)
	jq --arg now "$now" '.last_check = $now' "$state" >"$state.tmp" && mv "$state.tmp" "$state"
	exit 0
fi

since=$(jq -r .last_check "$state")
human='select(.body // "" | contains("<!-- claude -->") | not)'

# `since` matches any update, including Claude's own comments, so keep only
# issues created since; maintainer comments on older ones are listed below.
gh api "repos/$repo/issues?state=all&since=$since&per_page=100" --jq ".[] | $human |
	select(.created_at > \"$since\") |
	\"\(if .pull_request then \"PR\" else \"ISSUE\" end) #\(.number) [\(.state)] \(.title)\""
gh api "repos/$repo/issues/comments?since=$since&per_page=100" --jq ".[] | $human |
	\"COMMENT on #\(.issue_url | split(\"/\") | last): \(.html_url)\""
gh api "repos/$repo/pulls/comments?since=$since&per_page=100" --jq ".[] | $human |
	\"REVIEW COMMENT: \(.html_url)\""
gh api "repos/$repo/actions/runs?branch=main&status=failure&created=>$since" --jq '.workflow_runs[] |
	"FAILED RUN \(.name) on \(.head_sha[0:7]): \(.html_url)"'
