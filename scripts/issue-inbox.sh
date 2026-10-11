#!/usr/bin/env bash
# Prints what is new on GitHub for Claude since the last check: issues and
# comments from anyone but Claude's own account (desocketed)
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

# --done moves last_check to when the listing was taken, not to now, so
# nothing that arrived while the listing was being handled is skipped.
if [ "${1:-}" = "--done" ]; then
	jq '.last_check = (.listed_at // .last_check)' "$state" >"$state.tmp" && mv "$state.tmp" "$state"
	exit 0
fi

since=$(jq -r .last_check "$state")
now=$(date -u +%Y-%m-%dT%H:%M:%SZ)
jq --arg now "$now" '.listed_at = $now' "$state" >"$state.tmp" && mv "$state.tmp" "$state"
# Judged by author, not by the <!-- claude --> marker, which anyone can paste.
# The association (OWNER, COLLABORATOR, NONE, ...) says whose request it is.
human='select(.user.login != "desocketed")'
who='\(.user.login) (\(.author_association))'

# `since` matches any update, including Claude's own comments, so keep only
# issues created since; maintainer comments on older ones are listed below.
gh api --paginate "repos/$repo/issues?state=all&since=$since&per_page=100" --jq ".[] | $human |
	select(.created_at > \"$since\") |
	\"\(if .pull_request then \"PR\" else \"ISSUE\" end) #\(.number) [\(.state)] \(.title) by $who\""
gh api --paginate "repos/$repo/issues/comments?since=$since&per_page=100" --jq ".[] | $human |
	\"COMMENT on #\(.issue_url | split(\"/\") | last) by $who: \(.html_url)\""
gh api --paginate "repos/$repo/pulls/comments?since=$since&per_page=100" --jq ".[] | $human |
	\"REVIEW COMMENT by $who: \(.html_url)\""
gh api --paginate "repos/$repo/actions/runs?branch=main&status=failure&created=>$since" --jq '.workflow_runs[] |
	"FAILED RUN \(.name) on \(.head_sha[0:7]): \(.html_url)"'
