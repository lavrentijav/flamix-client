#!/bin/sh
# scripts/check-windows.sh — cross-check the Windows (x86_64-pc-windows-gnu) build from Linux.
# OWNER: task T16. Run it on the build server via scripts/ru1:
#
#   scripts/ru1 scripts/check-windows.sh
#
# Zig provides the Windows C toolchain/linker for crates that ship C code (ring); the repository
# itself contains no Zig code. Override the zig binary with ZIG.
set -e

TARGET="${FLAMIX_WIN_TARGET:-x86_64-pc-windows-gnu}"
ZIG="${ZIG:-zig}"
# Rust target triple -> zig target triple (zig has no `pc` vendor).
ZIG_TARGET="${FLAMIX_WIN_ZIG_TARGET:-x86_64-windows-gnu}"

command -v "$ZIG" >/dev/null 2>&1 || {
    echo "check-windows: '$ZIG' not found (install zig, e.g. into ~/flamix-tools/zig)" >&2
    exit 2
}

rustup target add "$TARGET" >/dev/null 2>&1 || true

# cc-rs (the version pulled in by ring) passes the Rust LLVM triple to the C compiler as
# `--target=x86_64-pc-windows-gnu`, but zig >= 0.14 only understands `-target x86_64-windows-gnu`.
# Wrap `zig cc`/`zig c++` to drop the Rust triple and add zig's own target, then hand the wrapper
# to cc-rs (cargo-zigbuild keeps a pre-set CC_* — see its `add_env_if_missing`).
wrapdir=$(mktemp -d)
trap 'rm -rf "$wrapdir"' EXIT INT TERM
zig_path=$(command -v "$ZIG")

make_wrapper() {
    out="$1"
    zigcmd="$2"
    cat > "$out" <<EOF
#!/bin/bash
args=()
for a in "\$@"; do
    case "\$a" in
        --target=*) ;;
        *) args+=("\$a") ;;
    esac
done
exec "$zig_path" $zigcmd -target "$ZIG_TARGET" "\${args[@]}"
EOF
    chmod +x "$out"
}

make_wrapper "$wrapdir/zigcc" cc
make_wrapper "$wrapdir/zigcxx" c++

env_target=$(printf '%s' "$TARGET" | tr '-' '_')
eval "export CC_${env_target}=\"$wrapdir/zigcc\""
eval "export CXX_${env_target}=\"$wrapdir/zigcxx\""

if command -v cargo-zigbuild >/dev/null 2>&1; then
    echo "==> cargo zigbuild --target $TARGET -p flamix"
    cargo zigbuild --target "$TARGET" -p flamix
else
    echo "==> cargo-zigbuild not found; falling back to cargo check (zig cc as CC)"
    cargo check --target "$TARGET" -p flamix
fi

# clippy never links, but the build scripts of C crates (ring) still need a cross C compiler.
echo "==> cargo clippy --target $TARGET --workspace --all-targets -- -D warnings"
cargo clippy --target "$TARGET" --workspace --all-targets -- -D warnings

echo "check-windows: OK ($TARGET)"
