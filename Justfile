# Justfile

# By default, run a full check (check)
default: check

# Full check: fmt (format check), clippy, tests
check:
    @echo "==> Checking format..."
    cargo fmt --all -- --check

    @echo "==> Checking clippy..."
    cargo clippy --all-targets --all-features -- -D warnings

    @echo "==> Running tests with nextest..."
    cargo nextest run --workspace --all-features

    @echo "==> Running audit..."
    cargo audit

    @echo "==> Checking outdated dependencies..."
    cargo outdated

    @echo "All checks passed!"

# Code formatting
fmt:
    @echo "==> Formatting code..."
    cargo fmt --all

# Running Clippy
clippy:
    @echo "==> Clippy linting..."
    cargo clippy --all-targets --all-features -- -D warnings

# Running tests (nextest)
test:
    @echo "==> Running tests (debug)..."
    cargo nextest run --workspace --all-features

# Running tests in release build
test-release:
    @echo "==> Running tests (release)..."
    cargo nextest run --workspace --all-features --release

# Generating (and opening) documentation
doc:
    @echo "==> Building docs..."
    cargo doc --no-deps --all-features --open

# Checking for outdated dependencies
outdated:
    @echo "==> Checking outdated dependencies..."
    cargo outdated

# Updating dependencies
update:
    @echo "==> Updating dependencies..."
    cargo update

# Checking dependencies for vulnerabilities
audit:
    @echo "==> Auditing dependencies..."
    cargo audit