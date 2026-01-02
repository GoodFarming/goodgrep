.PHONY: ci fmt check test clippy

ci:
	./Scripts/ci.sh

fmt:
	./Scripts/ggrep/cargo.sh +nightly fmt --manifest-path ./Tools/ggrep/Cargo.toml --all -- --check

check:
	./Scripts/ggrep/cargo.sh +nightly check --manifest-path ./Tools/ggrep/Cargo.toml --no-default-features

test:
	./Scripts/ggrep/cargo.sh +nightly test --manifest-path ./Tools/ggrep/Cargo.toml --no-default-features

clippy:
	./Scripts/ggrep/cargo.sh +nightly clippy --manifest-path ./Tools/ggrep/Cargo.toml --no-default-features
