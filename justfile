set dotenv-load
dev:
    cargo test --package oism-server --bin oism-server --all-features -- test::test_main_locally --exact --nocapture --ignored | bunyan
test-all:
    cargo test | bunyan

structure:
    d2 docs/project-structure.d2 docs/images/project-structure.png

watch-structure:
    d2 -w docs/project-structure.d2 docs/images/project-structure.png