.PHONY: test-shell test-workflows desktop-verify desktop-package-current

test-shell:
	hostile_root="$$(mktemp -d)"; \
	trap 'rm -rf "$${hostile_root}"' EXIT; \
	hostile_codex="$${hostile_root}/must-not-be-used"; \
	set -- ./tests/setup_*_test.sh; \
	for t in "$${@}"; do \
		[ -f "$${t}" ] || continue; \
		CODEX_HOME="$${hostile_codex}" bash "$${t}" || exit $$?; \
	done

test-workflows:
	bash ./tests/desktop_workflow_test.sh
	bash ./tests/agent_model_preset_workflow_test.sh
	bash ./tests/setup_release_packaging_test.sh
	bash ./tests/index_docs_test.sh

desktop-verify:
	cd desktop && npm run verify

desktop-package-current:
	cd desktop && npm run sidecars:build
	cd desktop && npm exec tauri -- build --target "$$(rustc --print host-tuple)" --no-sign --ci
	cd desktop && npm run package:verify -- "$$(rustc --print host-tuple)"
