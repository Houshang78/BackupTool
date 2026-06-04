# ساخت (Build)

**زبان‌ها:** [English](BUILDING.md) · [Deutsch](BUILDING.de.md) · فارسی

چگونه هر نسخه را روی هر سیستم‌عامل بسازیم. هر بسته **روی سیستم‌عامل خودش** ساخته
می‌شود — PyInstaller، dpkg و pkgbuild کراس‌کامپایل نمی‌کنند و باینری‌های راست بومیِ
هر OS/معماری هستند.

برای دانلودهای آماده معمولاً نیازی به ساخت نیست — به [INSTALL.fa.md](INSTALL.fa.md)
و بخش *Releases* در گیت‌هاب نگاه کنید.

---

## یک دستور برای هر سیستم‌عامل (توصیه‌شده)

اسکریپت‌های پوششی، باینری‌های **راست** **و** بستهٔ **پایتون** را برای همان OS
می‌سازند و همه را در `dist/<os>/` جمع می‌کنند:
```bash
bash   scripts/build-linux.sh                 # روی Debian/Ubuntu
bash   scripts/build-macos.sh                 # روی macOS
powershell -ExecutionPolicy Bypass -File scripts\build-windows.ps1   # روی Windows
```
نتیجه‌ها:
- `dist/linux/`  — `backuptool`، `backuptool-gui`، `*.deb`
- `dist/macos/`  — `backuptool`، `backuptool-gui`، `backuptool.app`، `*.pkg`
- `dist/windows/` — `backuptool.exe`، `backuptool-gui.exe`، `backuptool-python.exe`

---

## نسخهٔ راست

نیازمند **Rust ≥ 1.78** (فایل `Cargo.lock` قالب v4 است). نصب از <https://rustup.rs>؛
روی نسخهٔ قدیمی `rustup update stable`.
```bash
cd rust
cargo build --release --bin backuptool                       # CLI (یک باینری)
cargo build --release --features gui --bin backuptool-gui    # رابط گرافیکیِ Slint
cargo test                                                   # تست‌های واحد
cargo clippy --all-targets --features gui -- -D warnings     # لینت‌ها
```
روی **لینوکس** رابط گرافیکی به کتابخانه‌های سیستمی نیاز دارد:
```bash
sudo apt install -y build-essential pkg-config \
                    libfontconfig1-dev libxcb1-dev libxkbcommon-dev libwayland-dev
```
باینری‌ها در `rust/target/release/` قرار می‌گیرند و مستقل‌اند (بدون Python/Qt).

> «lock file version 4 was found…» → Cargo شما قدیمی‌تر از 1.78 است. `rustup update
> stable` را اجرا کنید یا `rm rust/Cargo.lock` تا یک lock سازگار ساخته شود.

---

## نسخهٔ پایتون

**CLI** فقط به Python 3 (کتابخانهٔ استاندارد) نیاز دارد. **رابط گرافیکی** به PySide6:
```bash
pip3 install PySide6
```
### اجرا از سورس / تست‌ها
```bash
cd python
./backuptool-portable                          # GUI (یا CLI با آرگومان)
python3 -m unittest discover -s tests -v        # تست‌ها
pip install .                                   # نصب توسعه: backuptool, backuptool-gui
```
### بسته‌های بومی
```bash
cd python
bash packaging/build-deb.sh                     # لینوکس  -> dist/backuptool_1.2.0_all.deb
bash packaging/build-macos.sh                   # مک      -> dist/backuptool.app + .pkg
powershell -File packaging\build-windows.ps1    # ویندوز  -> dist\backuptool.exe (PyInstaller)
```
`.deb`/`.pkg` به PySide6 به‌عنوان وابستگیِ زمان اجرا نیاز دارند (جداگانه یا با
`pip3 install PySide6`). فایل `.exe` ویندوز خودِ PySide6 را همراه دارد.

---

## انتشار (Release) با CI

پوش‌کردن یک تگ، `.github/workflows/release.yml` را اجرا می‌کند، همان آرتیفکت‌ها را
روی رانرهای لینوکس/مک/ویندوزِ گیت‌هاب می‌سازد و به Release می‌چسباند:
```bash
git tag v1.2.0
git push origin v1.2.0
```
پیش از ساختِ بسته‌های خودتان: نویسنده/ناشر و شناسهٔ بستهٔ مک را در
`python/pyproject.toml`، `python/packaging/*` و `rust/Cargo.toml` تغییر دهید.
