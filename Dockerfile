# Toolchain bump: edit rust-toolchain.toml only; this image reads the channel
# from it. LLVM_RELEASE_TAG tracks the laputa-systems/llvm-prebuilt-musl
# prereleases; bump it with the matching LLVM_* checksums below.
FROM alpine:3.24.2@sha256:294b683cb724975bec92580e1e685676bd4b50bda910ddb8c51d4cabeaec77e6
COPY --from=oci.17166969.xyz/owned-tools@sha256:1a5d1306bb105f21b60c67bbf04677a92048c02a808ae9b29dfdac1a90f2f073 /opt/laputa /opt/laputa
ENV PATH=/opt/laputa/bin:$PATH

ARG TARGETARCH
ARG LLVM_VERSION=23.1.5
ARG LLVM_RELEASE_TAG=release-b29e4d07a2b9a5e72319b37429cf375149212755
ARG GLUON_RELEASE_TAG=release-fa8b6aace38683a17ee3f398d8b568341c7192fe
ARG RUSTUP_VERSION=1.29.1
ARG RUSTUP_X86_64_SHA256=331228566cc931f32cd684f9bebc5956a5e5d7394c8d5de595f196a658ab98e6
ARG RUSTUP_AARCH64_SHA256=1ddf36182ac5d1782dbeefcb9bea7f5f9412d88f4a0d5047b1564a223624e8c6

RUN apk add --no-cache \
    binutils=2.45.1-r1 \
    bash=5.3.9-r1 \
    file=5.47-r2 \
    g++=15.2.0-r5 \
    gcc=15.2.0-r5 \
    make=4.4.1-r4 \
    procps \
    strace=6.19-r1 \
    musl-dev=1.2.6-r3 \
    libgcc=15.2.0-r5 \
    tzdata=2026f-r0

# rustix requests libutil for Unix PTY support; musl provides those symbols in
# libc, so an empty archive satisfies the legacy library name without adding a
# glibc runtime dependency.
RUN ar rcs /usr/lib/libutil.a

# LLVM tools must be musl-linked to run in this Alpine image.
RUN case "$TARGETARCH" in \
        amd64) arch=x86_64; sum=de3f6d0ad089391482147c61648c511c500b25289132fe7f2c446a0905406d2a ;; \
        arm64) arch=aarch64; sum=4d8b637d65ed0299c8b7852c1e7e1d41f7141346ce31e707f0975ff67460121e ;; \
        *) echo "unsupported TARGETARCH: $TARGETARCH" >&2; exit 1 ;; \
    esac \
    && archive=/kache/.work/tmp/llvm.tar.xz \
    && mkdir -p /kache/.work/tmp \
    && wget -O "$archive" "https://github.com/laputa-systems/llvm-prebuilt-musl/releases/download/${LLVM_RELEASE_TAG}/clang+llvm-${LLVM_VERSION}-${arch}-linux-musl.tar.xz" \
    && echo "$sum  $archive" | sha256sum -c - \
    && mkdir -p /opt/llvm-musl \
    && tar xf "$archive" -C /opt/llvm-musl --strip-components=1 \
    && test -f /opt/llvm-musl/lib/libclang.so \
    && rm "$archive"

# Gluon is the canonical ninja/meson executor; pin the musl prerelease
# like LLVM above. Static binaries, so they run in the musl image as-is.
RUN case "$TARGETARCH" in \
        amd64) arch=x86_64; sum=3aacbd8756a4dcfeff9cd139ebcd5a3b4cb2cc433c599b7e21f446c716987ce8 ;; \
        arm64) arch=aarch64; sum=6b352c03399cf41f45ee5d9ff601ea17cf4768f74aff31a164e145e6e2326e56 ;; \
        *) echo "unsupported TARGETARCH: $TARGETARCH" >&2; exit 1 ;; \
    esac \
    && archive=/kache/.work/tmp/gluon.tar.xz \
    && wget -O "$archive" "https://github.com/laputa-systems/gluon/releases/download/${GLUON_RELEASE_TAG}/gluon-${arch}-unknown-linux-musl.tar.xz" \
    && echo "$sum  $archive" | sha256sum -c - \
    && mkdir -p /opt/gluon \
    && tar -xJf "$archive" -C /opt/gluon \
    && ln -s /opt/gluon/ninja /usr/local/bin/ninja \
    && ln -s /opt/gluon/meson /usr/local/bin/meson \
    && ln -s /opt/gluon/gluon-build /usr/local/bin/gluon-build \
    && ninja --version \
    && rm "$archive"

# rustc's default linker driver is `cc`, and several tests probe or run it;
# the prebuilt LLVM tree only ships `clang` under its own name. Without the
# conventional names, native links and the tests that exercise them fail with
# "linker `cc` not found".
# Alpine's gcc/g++ ship /usr/bin/cc and /usr/bin/c++; use -sfn so the LLVM
# symlinks win and cc keeps resolving to LLVM clang.
RUN ln -s /opt/llvm-musl/bin/clang /usr/local/bin/cc \
    && ln -s /opt/llvm-musl/bin/clang++ /usr/local/bin/c++ \
    && ln -sfn /opt/llvm-musl/bin/clang /usr/bin/cc \
    && ln -sfn /opt/llvm-musl/bin/clang++ /usr/bin/c++ \
    && cc --version

RUN for target in x86_64-unknown-linux-musl aarch64-unknown-linux-musl; do \
        stub_dir="/usr/lib/e-crt/$target"; \
        mkdir -p "$stub_dir"; \
        for obj in crtbegin.o crtbeginS.o crtbeginT.o crtend.o crtendS.o; do \
            /opt/llvm-musl/bin/clang --target="$target" -x c -c /dev/null -o "$stub_dir/$obj"; \
        done; \
    done

# Cargo-driven links get `-B/usr/lib/e-crt/<target>` from musl-cargo, but a
# bare `cc` or `rustc` invocation does not. The compiler CRT stubs for the
# image's native arch must also sit on clang's default search path, beside
# musl-dev's crt1.o/crti.o/crtn.o; otherwise raw links fail with
# "cannot open crtbeginS.o".
RUN case "$TARGETARCH" in \
        amd64) stub_target=x86_64-unknown-linux-musl ;; \
        arm64) stub_target=aarch64-unknown-linux-musl ;; \
        *) echo "unsupported TARGETARCH: $TARGETARCH" >&2; exit 1 ;; \
    esac \
    && for obj in crtbegin.o crtbeginS.o crtbeginT.o crtend.o crtendS.o; do \
        ln -s "/usr/lib/e-crt/$stub_target/$obj" "/usr/lib/$obj"; \
    done

RUN case "$TARGETARCH" in \
        amd64) arch=x86_64; sum=$RUSTUP_X86_64_SHA256 ;; \
        arm64) arch=aarch64; sum=$RUSTUP_AARCH64_SHA256 ;; \
        *) echo "unsupported TARGETARCH: $TARGETARCH" >&2; exit 1 ;; \
    esac \
    && init=/kache/.work/tmp/rustup-init \
    && wget -O "$init" "https://static.rust-lang.org/rustup/archive/${RUSTUP_VERSION}/${arch}-unknown-linux-musl/rustup-init" \
    && echo "$sum  $init" | sha256sum -c - \
    && chmod +x "$init" \
    && "$init" -y --default-toolchain none \
    && rm "$init"

ENV PATH="/opt/llvm-musl/bin:/root/.cargo/bin:$PATH" \
    CC="/opt/llvm-musl/bin/clang" \
    AR="/opt/llvm-musl/bin/llvm-ar" \
    RANLIB="/opt/llvm-musl/bin/llvm-ranlib" \
    LIBRARY_PATH="/opt/llvm-musl/lib" \
    LIBCLANG_PATH="/opt/llvm-musl/lib" \
    CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER="/opt/llvm-musl/bin/clang" \
    CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER="/opt/llvm-musl/bin/clang"

COPY rust-toolchain.toml /tmp/rust-toolchain.toml
RUN channel="$(sed -n 's/^channel *= *"\(.*\)"/\1/p' /tmp/rust-toolchain.toml)" \
    && test -n "$channel" \
    && rustup toolchain install "$channel" \
        --target x86_64-unknown-linux-musl \
        --target aarch64-unknown-linux-musl \
        --component rust-src \
    && rustup default "$channel" \
    && rm /tmp/rust-toolchain.toml

# Host proc-macro links search rustc's target library directory for CRT
# startup objects. Keep those objects available there without changing the
# target's build-std Rust flags.
RUN host_target="$(rustc -vV | awk '/^host:/ {print $2}')" \
    && host_libdir="$(rustc --print target-libdir)" \
    && for obj in crtbegin.o crtbeginS.o crtbeginT.o crtend.o crtendS.o; do \
        ln -sf "/usr/lib/e-crt/$host_target/$obj" "$host_libdir/$obj"; \
    done

# Alpine keeps these libraries outside the Rust musl target directory.
RUN host_libdir="$(rustc --print target-libdir)" \
    && ln -sf /usr/lib/libgcc_s.so.1 "$host_libdir/libgcc_s.so" \
    && ln -sf /usr/lib/libgcc_s.so.1 "$host_libdir/libgcc_s.so.1" \
    && ln -sf /usr/lib/libc.so "$host_libdir/libc.so"

RUN cat > /usr/local/bin/musl-cargo <<'EOF' \
    && chmod +x /usr/local/bin/musl-cargo
#!/bin/sh
set -eu

target=${MUSL_TARGET:?MUSL_TARGET is required}
case "$target" in
    x86_64-unknown-linux-musl)
        crt=/usr/lib/e-crt/x86_64-unknown-linux-musl
        ;;
    aarch64-unknown-linux-musl)
        crt=/usr/lib/e-crt/aarch64-unknown-linux-musl
        ;;
    *)
        exec cargo "$@"
        ;;
esac

mode=${MUSL_BUILD_MODE:-static}
native="-L native=/usr/lib -Clink-arg=-B$crt"
release="-Cpanic=abort"
case "$mode" in
    static|test|driver)
        flags=$native
        ;;
    release-static)
        flags="$native $release -Ctarget-feature=+crt-static"
        ;;
    *)
        echo "musl-cargo: unknown build mode: $mode" >&2
        exit 2
        ;;
    esac

target_env=$(printf '%s' "$target" | tr '[:lower:]-' '[:upper:]_')
linker_var="CARGO_TARGET_${target_env}_LINKER"
rustflags_var="CARGO_TARGET_${target_env}_RUSTFLAGS"
export "$linker_var=clang"
if [ "$mode" = driver ] || [ "$mode" = test ]; then
    # Cargo builds proc-macro and test-support crates for the container host,
    # where target-scoped flags do not apply. Keep those links on the same
    # musl CRT path as the instrumented binary.
    unset CARGO_ENCODED_RUSTFLAGS
    export RUSTFLAGS="$flags"
else
    export "$rustflags_var=$flags"
fi
exec cargo "$@"
EOF
