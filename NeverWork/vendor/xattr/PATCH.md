# xattr 0.2.3 本地补丁

上游：<https://github.com/Stebalien/xattr>（crates.io `xattr 0.2.3`，MIT/Apache-2.0）。

## 原因

`xattr` 是 `gpui_http_client → zed-async-tar` 的传递依赖。0.2.3 的
`src/sys/mod.rs` 对 linux/macos 统一写 `pub const ENOATTR = libc::ENOATTR`，
但 Linux 的 libc 没有 `ENOATTR`（glibc 在 `attr/xattr.h` 里把它别名到 `ENODATA`），
GitHub Actions 的 ubuntu 构建因此报 E0425，Linux 二进制产不出来。

上游没有发布修复版本，`zed-async-tar 0.5.0-zed` 又要求 `xattr ^0.2`，
没法直接升级到 1.x，所以在 workspace 用 `[patch.crates-io]` 指向本目录。

## 改动

只改 `src/sys/mod.rs`：

- 把 `ENOATTR` 从 `platforms!` 的按平台组循环里提到宏体外，只定义一次
  （否则 linux 上 android/linux/macos 与 freebsd/netbsd 两组会各自展开一份，报 E0428）
- linux / android：`ENOATTR = libc::ENODATA`
- macos / freebsd / netbsd：`ENOATTR = libc::ENOATTR`（保持原样）

语义等价：`getxattr` 在属性不存在时返回 `ENODATA`。

## 何时删掉

`zed-async-tar` 升级到依赖 `xattr 1.x` 后，删除本目录与
`NeverWork/Cargo.toml` 里的 `[patch.crates-io]` 配置。
