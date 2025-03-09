# Maintainer: Amirhossein Ghanipour d3v1ll3n@gmail.com
pkgname=archlink
pkgver=0.1.0
pkgrel=1
pkgdesc="ArchLink helps Arch Linux users to find the right packages to install"
arch=('x86_64')
url="https://github.com/amirhosseinghanipour/archlink"
license=('MIT')
depends=('pacman')
optdepends=('yay: for installing AUR packages'
            'paru: for installing AUR packages')
makedepends=('cargo' 'git')  
source=("git+$url.git#tag=v$pkgver")
sha256sums=('SKIP')  

prepare() {
  cd "$srcdir/$pkgname"
  cargo fetch --locked --target "$CARCH-unknown-linux-gnu"
}

build() {
  cd "$srcdir/$pkgname"
  cargo build --release --locked --target "$CARCH-unknown-linux-gnu"
}

package() {
  cd "$srcdir/$pkgname"
  install -Dm755 "target/$CARCH-unknown-linux-gnu/release/$pkgname" "$pkgdir/usr/bin/$pkgname"
  install -Dm644 "README.md" "$pkgdir/usr/share/doc/$pkgname/README.md"
  install -Dm644 "LICENSE" "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
  install -Dm644 <(echo -e "[default]\nmax_results = 10") "$pkgdir/etc/archlink/config.toml"
}
