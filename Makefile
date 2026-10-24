.DEFAULT_GOAL := help
.PHONY: help references-mainline references-cluster

help:
	@echo "make references-mainline  move references/herdr to the upstream master tip"
	@echo "make references-cluster   start four agents to update easy-herdr from the current references/herdr"

references-mainline:
	git submodule update --init --remote -- references/herdr
	git submodule status -- references/herdr

# The prompt stays on one line: easy-herdr types it, and a newline would submit it early.
references-cluster:
	@command -v easy-herdr >/dev/null 2>&1 || { \
		echo "easy-herdr is not installed. Download it from https://github.com/scott1028/easy-herdr/releases and put it on PATH." >&2; \
		exit 1; \
	}
	@printf 'Command for planner, executor and reviewer1 (default: claude): '; read -r team_cli; team_cli=$${team_cli:-claude}; \
	printf 'Command for reviewer2 (default: codex): '; read -r reviewer2_cli; reviewer2_cli=$${reviewer2_cli:-codex}; \
	echo "easy-herdr cluster $$team_cli $$team_cli $$team_cli $$reviewer2_cli"; \
	easy-herdr cluster "$$team_cli" "$$team_cli" "$$team_cli" "$$reviewer2_cli" -- \
		"You are the planner. Read references/CLUSTER.md and follow it. planner, executor and reviewer1 run $$team_cli; reviewer2 runs $$reviewer2_cli."
