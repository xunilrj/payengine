r: run
run:
    RUST_LOG=trace cargo run -- tests/ok/simple.csv

accept-tests:
    cargo t || cargo insta accept
