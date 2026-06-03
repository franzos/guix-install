(use-modules (guix packages)
             (guix search-paths)
             (gnu packages rust)
             (gnu packages commencement)
             (gnu packages tls)
             (gnu packages base)
             (gnu packages pkg-config)
             (gnu packages freedesktop)
             (gnu packages xdisorg)
             (gnu packages xml)
             (gnu packages fontutils)
             (gnu packages gl))

(define openssl-with-dir
  (package
    (inherit openssl)
    (native-search-paths
     (cons (search-path-specification
            (variable "OPENSSL_DIR")
            (files '("."))
            (file-type 'directory)
            (separator #f))
           (package-native-search-paths openssl)))))

(define gcc-toolchain-with-cc
  (package
    (inherit gcc-toolchain)
    (native-search-paths
     (cons (search-path-specification
            (variable "CC")
            (files '("bin/gcc"))
            (file-type 'regular)
            (separator #f))
           (package-native-search-paths gcc-toolchain)))))

(packages->manifest
 (list rust
       (list rust "cargo")
       rust-analyzer
       gcc-toolchain-with-cc
       openssl-with-dir
       ;; GUI (iced 0.14, tiny-skia software renderer) build + runtime deps
       pkg-config
       wayland
       libxkbcommon
       expat
       fontconfig
       freetype
       mesa
       libglvnd))
