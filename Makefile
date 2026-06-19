.PHONY: test-shell

test-shell:
	set -- ./tests/setup_*_test.sh; \
	for t in "$${@}"; do \
		[ -f "$${t}" ] || continue; \
		bash "$${t}"; \
	done
