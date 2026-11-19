macro_rules! platforms {
    ($($($platform:expr);* => $module:ident),*) => {
        $(
            #[cfg(any($(target_os = $platform),*))]
            #[cfg_attr(not(any($(target_os = $platform),*)), allow(dead_code))]
            mod $module;

            #[cfg(any($(target_os = $platform),*))]
            pub use self::$module::*;
        )*

        // NeverWork patch：Linux/Android 的 libc 没有 ENOATTR（glibc 在 attr/xattr.h
        // 里把它别名到 ENODATA），上游 0.2.3 在按组展开的宏体里直接引用
        // libc::ENOATTR 会编译失败。这里只定义一次，避免多个平台组各展开一份
        // 造成 E0428 重复定义。
        #[cfg(any(target_os = "android", target_os = "linux"))]
        pub const ENOATTR: ::libc::c_int = ::libc::ENODATA;

        #[cfg(any(target_os = "macos", target_os = "freebsd", target_os = "netbsd"))]
        pub const ENOATTR: ::libc::c_int = ::libc::ENOATTR;

        #[cfg(all(feature = "unsupported", not(any($($(target_os = $platform),*),*))))]
        #[cfg_attr(any($($(target_os = $platform),*),*), allow(dead_code))]
        mod unsupported;

        #[cfg(all(feature = "unsupported", not(any($($(target_os = $platform),*),*))))]
        pub use self::unsupported::*;
        #[cfg(all(feature = "unsupported", not(any($($(target_os = $platform),*),*))))]
        pub const ENOATTR: ::libc::c_int = 0;


        /// A constant indicating whether or not the target platform is supported.
        ///
        /// To make programmer's lives easier, this library builds on all platforms.
        /// However, all function calls on unsupported platforms will return
        /// `io::Error`s.
        ///
        /// Note: If you would like compilation to simply fail on unsupported platforms,
        /// turn of the `unsupported` feature.
        pub const SUPPORTED_PLATFORM: bool = cfg!(any($($(target_os = $platform),*),*));
    }
}

platforms! {
    "android"; "linux"; "macos" => linux_macos,
    "freebsd"; "netbsd" => bsd
}
