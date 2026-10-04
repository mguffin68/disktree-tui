BINS := target/release/disktree-tui

run:
	cargo run --release -- $(filter-out $@,$(MAKECMDGOALS))

release:
	cargo build --release

test:
	cargo test --workspace

lint:
	cargo fmt --check
	cargo clippy --workspace -- -D warnings

ci: lint test

install: release
	install -d ~/.local/bin
	install $(BINS) ~/.local/bin/

uninstall:
	rm -f ~/.local/bin/disktree-tui

clean:
	cargo clean

$(MAKECMDGOALS):
	@:
