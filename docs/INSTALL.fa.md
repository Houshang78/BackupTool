# نصب و اجرا

**زبان‌ها:** [English](INSTALL.md) · [Deutsch](INSTALL.de.md) · فارسی

دو راه: **اجرا بدون نصب** (پرتابل) یا **نصبِ** یک بستهٔ بومی. سپس
[USAGE.fa.md](USAGE.fa.md) نحوهٔ گرفتن پشتیبان را نشان می‌دهد.

---

## الف) اجرا بدون نصب (پرتابل)

### باینری راست (یک فایل، بدون وابستگی)
پس از `cargo build --release` (نگاه کنید به [BUILDING.fa.md](BUILDING.fa.md)) یا از
یک آرشیو Release:
```bash
# از یک Build
./rust/target/release/backuptool --help
./rust/target/release/backuptool-gui            # رابط گرافیکی بومی
# از یک آرشیو Release
tar -xzf backuptool-v1.2.3-linux-x86_64.tar.gz  # یا unzip فایل .zip ویندوز
./backuptool --help
./backuptool-gui
```
باینری را می‌توان هرجا (حتی روی دیسک پشتیبان) کپی و مستقیماً اجرا کرد.

### اجراکنندهٔ پرتابلِ پایتون
CLI فقط به Python 3 نیاز دارد؛ رابط گرافیکی به‌علاوهٔ `pip3 install PySide6`.
```bash
cd python
./backuptool-portable                                   # GUI
./backuptool-portable backup ~/Documents -d /Volumes/Backup -s my-laptop --progress
```
`backuptool-portable` خودش `PYTHONPATH` را تنظیم و `python3 -m backuptool` را صدا
می‌زند. می‌توانید کل پوشهٔ `python/` را روی دیسک کپی و از همان‌جا اجرا کنید.

در ویندوز، خروجی PyInstaller یک فایل پرتابلِ واحد است — کافی است `backuptool.exe` را
اجرا کنید (دابل‌کلیک = GUI، با آرگومان = CLI).

---

## ب) نصب یک بستهٔ بومی

### لینوکس (.deb)
```bash
sudo apt install ./backuptool_1.2.3_all.deb
backuptool --help          # CLI
backuptool gui             # GUI (یا ورودی منوی «backuptool»)
```
برای GUI به `python3-pyside6` نیاز است (`sudo apt install python3-pyside6` یا
`pip3 install PySide6`). حذف: `sudo apt remove backuptool`.

### مک (.pkg یا .app)
```bash
sudo installer -pkg backuptool-1.2.3.pkg -target /     # CLI در /usr/local/bin
backuptool --help
```
یا روی `backuptool.app` دابل‌کلیک کنید (GUI). در هر دو حالت یک‌بار:
`pip3 install PySide6`. حذف: `/usr/local/bin/backuptool` و `/usr/local/lib/backuptool`
را پاک کنید (و اپ را اگر به /Applications کپی کرده‌اید).

### ویندوز (.exe / Setup)
- پرتابل: کافی است `backuptool.exe` را نگه دارید و اجرا کنید.
- نصب‌کننده: `backuptool-setup-1.2.3.exe` (Inno Setup) را اجرا کنید — ورودیِ منوی
  استارت، آیکن دسکتاپِ اختیاری و افزودنِ اختیاری CLI به `PATH`. حذف از
  *Settings → Apps*.

---

## کدام باینری چه‌کار می‌کند؟

| نام | نسخه | کاربرد |
|---|---|---|
| `backuptool` | CLI | اسکریپت / cron / SSH |
| `backuptool-gui` (راست) / `backuptool gui` (پایتون) | GUI | با ماوس |
| `backuptool.exe` / `backuptool-python.exe` | ویندوز | Build راست / پایتون |

---

## نخستین اجرا

```bash
backuptool --version
backuptool list -d /Volumes/Backup       # تا اولین پشتیبان خالی است
# یک پشتیبانِ نخستینِ ایمن (اجرای آزمایشی)
backuptool backup ~/Documents -d /Volumes/Backup -s my-laptop -n
```
بعدی: [USAGE.fa.md](USAGE.fa.md) — مسیرها، استثناها، رمزنگاری، افزایشی و بازگردانی.
