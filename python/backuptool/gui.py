# SPDX-License-Identifier: GPL-3.0-or-later
"""Qt6 (PySide6) GUI – cross-platform (Linux/macOS/Windows), multilingual.

The core engine (core.py) is GUI-independent. UI strings are loaded from JSON
catalogs via the i18n module, with a language selector (EN/DE/FA and any extra
catalog dropped into lang/). Right-to-left languages (e.g. Persian) switch the
layout direction automatically.

Requires:  pip install PySide6
"""
from __future__ import annotations

import functools
import os
import socket
import sys

try:
    from PySide6.QtCore import Qt, QThread, QObject, Signal, Slot
    from PySide6.QtWidgets import (
        QApplication, QWidget, QMainWindow, QTabWidget, QVBoxLayout, QHBoxLayout,
        QGridLayout, QGroupBox, QLabel, QLineEdit, QPushButton, QListWidget,
        QSpinBox, QCheckBox, QComboBox, QProgressBar, QPlainTextEdit, QFileDialog,
        QMessageBox,
    )
except ImportError as e:  # handled by __main__ -> falls back to the CLI
    raise ImportError("PySide6 is not installed. Run:  pip install PySide6") from e

from . import __version__, core, crypto, discover, dbdump, reset
from .i18n import Translator, available


def _decommission_job(defaults=None, def_pw=None, **kw):
    """Worker entry point: evacuate, then optionally restore factory defaults,
    then log the manual reset instructions."""
    rep = core.evacuate(**kw)
    log = kw.get("log", print)
    if defaults and not kw.get("dry_run") and rep.get("delete_errors", 0) == 0:
        ddir, dset, dtgt = defaults
        core.restore(backup_dir=ddir, setname=dset, target=dtgt,
                     workers=kw.get("workers"), reapply_meta=True, dry_run=False,
                     passphrase=def_pw, log=log, progress=kw.get("progress"))
    log(reset.instructions())
    return rep


def _clone_job(argv=None, dry=False, **kw):
    """Worker entry point: run (or preview) a clone command."""
    log = kw.get("log", print)
    if dry:
        log("[dry-run] would run: " + " ".join(argv))
        return 0
    log("Running: " + " ".join(argv))
    code = reset.run_command(argv)
    log(f"clone exited with code {code}")
    return code


class Worker(QObject):
    """Runs a core function in its own QThread and reports via signals."""

    log = Signal(str)
    progress = Signal(int, int)
    finished = Signal(str)
    failed = Signal(str)

    def __init__(self, func, kwargs, label):
        super().__init__()
        self._func = func
        self._kwargs = kwargs
        self._label = label

    @Slot()
    def run(self):
        try:
            self._kwargs["log"] = lambda m: self.log.emit(str(m))
            self._kwargs["progress"] = lambda d, t, p: self.progress.emit(d, t)
            self._func(**self._kwargs)
            self.finished.emit(self._label)
        except Exception as e:  # noqa: BLE001
            self.failed.emit(str(e))


class MainWindow(QMainWindow):
    def __init__(self):
        super().__init__()
        self.tr_ = Translator("en")
        self._thread = None
        self._worker = None
        self.resize(820, 700)

        central = QWidget()
        self.setCentralWidget(central)
        outer = QVBoxLayout(central)

        # Language selector (top bar)
        topbar = QHBoxLayout()
        self.lbl_language = QLabel()
        self.lang_combo = QComboBox()
        self._langs = available()
        for code in self._langs:
            self.lang_combo.addItem(self.tr_.name_of(code), code)
        self.lang_combo.currentIndexChanged.connect(self._change_language)
        topbar.addStretch(1)
        topbar.addWidget(self.lbl_language)
        topbar.addWidget(self.lang_combo)
        outer.addLayout(topbar)

        self.tabs = QTabWidget()
        self.tabs.addTab(self._build_backup_tab(), "")
        self.tabs.addTab(self._build_restore_tab(), "")
        self.tabs.addTab(self._build_decommission_tab(), "")
        self.tabs.addTab(self._build_clone_tab(), "")
        outer.addWidget(self.tabs)

        self.progress = QProgressBar()
        outer.addWidget(self.progress)
        self.status = QLabel()
        outer.addWidget(self.status)
        self.logbox = QPlainTextEdit()
        self.logbox.setReadOnly(True)
        self.logbox.setMaximumBlockCount(5000)
        outer.addWidget(self.logbox, 1)

        self.retranslate()

    # ----------------------------------------------------------- Backup tab
    def _build_backup_tab(self) -> QWidget:
        w = QWidget()
        lay = QVBoxLayout(w)

        self.gb_sources = QGroupBox()
        gl = QVBoxLayout(self.gb_sources)
        self.sources = QListWidget()
        self.sources.setSelectionMode(QListWidget.ExtendedSelection)
        gl.addWidget(self.sources)
        row = QHBoxLayout()
        self.btn_add_dir = QPushButton(); self.btn_add_dir.clicked.connect(self._add_dir)
        self.btn_add_file = QPushButton(); self.btn_add_file.clicked.connect(self._add_file)
        self.btn_remove = QPushButton(); self.btn_remove.clicked.connect(self._rm_src)
        self.btn_auto = QPushButton(); self.btn_auto.clicked.connect(self._auto_sources)
        row.addWidget(self.btn_add_dir); row.addWidget(self.btn_add_file)
        row.addWidget(self.btn_remove); row.addWidget(self.btn_auto); row.addStretch(1)
        gl.addLayout(row)
        lay.addWidget(self.gb_sources)

        grid = QGridLayout()
        self.lbl_dest = QLabel()
        grid.addWidget(self.lbl_dest, 0, 0)
        self.dest = QLineEdit()
        _auto_dest = discover.default_destination() or core.suggested_dest()
        if _auto_dest:
            self.dest.setText(_auto_dest)   # USB / external / network if found, else suggested
        grid.addWidget(self.dest, 0, 1)
        self.btn_dest = QPushButton(); self.btn_dest.clicked.connect(lambda: self._pick_dir(self.dest))
        grid.addWidget(self.btn_dest, 0, 2)

        self.lbl_set = QLabel()
        grid.addWidget(self.lbl_set, 1, 0)
        self.setname = QLineEdit(socket.gethostname())
        grid.addWidget(self.setname, 1, 1)
        self.lbl_set_hint = QLabel()
        grid.addWidget(self.lbl_set_hint, 1, 2)

        self.lbl_workers = QLabel()
        grid.addWidget(self.lbl_workers, 2, 0)
        self.workers = QSpinBox(); self.workers.setRange(1, 64)
        self.workers.setValue(core.default_workers())
        grid.addWidget(self.workers, 2, 1)

        self.lbl_excl = QLabel()
        grid.addWidget(self.lbl_excl, 3, 0)
        self.excl = QLineEdit()
        grid.addWidget(self.excl, 3, 1, 1, 2)

        self.lbl_extra = QLabel()
        grid.addWidget(self.lbl_extra, 4, 0)
        self.extra = QLineEdit()
        self.extra.setPlaceholderText("/path/x, /path/y/file.db")
        grid.addWidget(self.extra, 4, 1, 1, 2)

        self.lbl_uid = QLabel()
        grid.addWidget(self.lbl_uid, 5, 0)
        self.uidf = QLineEdit()
        self.uidf.setPlaceholderText("postgres, 1001")
        grid.addWidget(self.uidf, 5, 1, 1, 2)
        lay.addLayout(grid)

        self.cb_checksum = QCheckBox()
        self.cb_delete = QCheckBox()
        self.cb_system = QCheckBox()
        self.cb_dry = QCheckBox()
        self.cb_vss = QCheckBox()
        lay.addWidget(self.cb_checksum); lay.addWidget(self.cb_delete)
        lay.addWidget(self.cb_system); lay.addWidget(self.cb_dry); lay.addWidget(self.cb_vss)

        # Databases: dump running DBs (+ optional connection) and a generic command.
        self.gb_db = QGroupBox()
        dbg = QGridLayout(self.gb_db)
        self.cb_databases = QCheckBox()
        dbg.addWidget(self.cb_databases, 0, 0, 1, 4)
        dbg.addWidget(QLabel("Host"), 1, 0)
        self.db_host = QLineEdit(); dbg.addWidget(self.db_host, 1, 1)
        dbg.addWidget(QLabel("Port"), 1, 2)
        self.db_port = QLineEdit(); dbg.addWidget(self.db_port, 1, 3)
        dbg.addWidget(QLabel("User"), 2, 0)
        self.db_user = QLineEdit(); dbg.addWidget(self.db_user, 2, 1)
        dbg.addWidget(QLabel("Pass"), 2, 2)
        self.db_pass = QLineEdit(); self.db_pass.setEchoMode(QLineEdit.Password)
        dbg.addWidget(self.db_pass, 2, 3)
        self.lbl_db_cmd = QLabel()
        dbg.addWidget(self.lbl_db_cmd, 3, 0)
        self.db_cmd = QLineEdit(); self.db_cmd.setPlaceholderText("oracle=expdp ...")
        dbg.addWidget(self.db_cmd, 3, 1, 1, 3)
        lay.addWidget(self.gb_db)

        self.btn_start_backup = QPushButton()
        self.btn_start_backup.clicked.connect(self._start_backup)
        lay.addWidget(self.btn_start_backup)
        lay.addStretch(1)
        return w

    def _add_dir(self):
        d = QFileDialog.getExistingDirectory(self, self.tr_("choose"))
        if d:
            self.sources.addItem(d)

    def _add_file(self):
        p, _ = QFileDialog.getOpenFileName(self, self.tr_("choose"))
        if p:
            self.sources.addItem(p)

    def _rm_src(self):
        for it in self.sources.selectedItems():
            self.sources.takeItem(self.sources.row(it))

    def _auto_sources(self):
        """Add auto-detected user/service/data directories to the source list."""
        existing = {self.sources.item(i).text() for i in range(self.sources.count())}
        added = 0
        for c in discover.user_data_sources():
            if c["kind"] in ("user", "service", "data") and c["path"] not in existing:
                self.sources.addItem(c["path"])
                existing.add(c["path"])
                added += 1
        self.status.setText(f"+{added}")

    def _pick_dir(self, line: QLineEdit):
        d = QFileDialog.getExistingDirectory(self, self.tr_("choose"))
        if d:
            line.setText(d)

    def _resolve_overlaps(self, sources):
        """Warn about sources already covered by another; return the kept list
        (or None if the user cancelled)."""
        overlaps = core.analyze_overlaps(sources)
        if not overlaps:
            return sources
        lines = []
        for o in overlaps[:20]:
            sample = ", ".join(core.brief_listing(o["path"], 8))
            lines.append(f"• {o['path']}  ⊂  {o['covered_by']}"
                         + (f"\n    [{sample}]" if sample else ""))
        box = QMessageBox(self)
        box.setIcon(QMessageBox.Question)
        box.setWindowTitle(self.tr_("overlap_title"))
        box.setText(self.tr_("overlap_msg"))
        box.setInformativeText("\n".join(lines))
        remove_btn = box.addButton(self.tr_("overlap_remove"), QMessageBox.AcceptRole)
        box.addButton(self.tr_("overlap_keep"), QMessageBox.RejectRole)
        box.addButton(QMessageBox.Cancel)
        box.exec()
        clicked = box.clickedButton()
        if clicked is None or box.standardButton(clicked) == QMessageBox.Cancel:
            return None
        if clicked is remove_btn:
            redundant = {o["path"] for o in overlaps}
            return [s for s in sources if s not in redundant]
        return sources  # keep all

    def _backup_then_db(self, log, progress, core_kwargs, specs, conn, out_dir, do_vss=False):
        """Optionally VSS-snapshot, run the file backup, then dump the requested databases."""
        from . import vss as _vss
        kw = dict(core_kwargs)
        shadow_ids = []
        if do_vss and not kw.get("dry_run"):
            try:
                src, vmap, shadow_ids = _vss.prepare(kw.get("sources", []))
                for dev, vol in vmap:
                    log(f"VSS snapshot of {vol}: {dev.rstrip(chr(92))}")
                kw["sources"] = src
                kw["vss_map"] = vmap
            except Exception as e:  # noqa: BLE001
                log(f"VSS unavailable ({e}); continuing without snapshot.")
        try:
            res = core.backup(log=log, progress=progress, **kw)
        finally:
            for sid in shadow_ids:
                _vss.remove(sid)
        if specs and not kw.get("dry_run"):
            log(f"Dumping {len(specs)} database(s) -> {out_dir}")
            dbdump.dump_all(specs, out_dir, log=log, conn=conn)
        return res

    def _start_backup(self):
        sources = [self.sources.item(i).text() for i in range(self.sources.count())]
        for p in (x.strip() for x in self.extra.text().replace(";", ",").split(",")):
            if p and p not in sources:
                sources.append(p)
        for u in (x.strip() for x in self.uidf.text().replace(";", ",").split(",")):
            if u:
                d = discover.resolve_uid(u)
                if d and d not in sources:
                    sources.append(d)
        if not sources:
            QMessageBox.warning(self, self.tr_("missing"), self.tr_("need_source"))
            return
        if not self.dest.text():
            QMessageBox.warning(self, self.tr_("missing"), self.tr_("need_dest"))
            return
        sources = self._resolve_overlaps(sources)
        if sources is None:
            return
        excludes = [x.strip() for x in self.excl.text().split(",") if x.strip()]
        kwargs = dict(
            sources=sources, dest=self.dest.text(),
            setname=self.setname.text() or None, workers=self.workers.value(),
            use_checksum=self.cb_checksum.isChecked(), extra_excludes=excludes,
            prune=self.cb_delete.isChecked(), dry_run=self.cb_dry.isChecked(),
            include_system=self.cb_system.isChecked(),
        )
        # Warn / offer to close apps that hold files open.
        from . import procs
        import time
        blockers = procs.running_blockers()
        if blockers:
            t = self.tr_
            box = QMessageBox(self)
            box.setIcon(QMessageBox.Warning)
            box.setWindowTitle(t("apps_title"))
            box.setText(t("apps_msg") + "\n\n" + ", ".join(blockers))
            b_close = box.addButton(t("close_apps"), QMessageBox.AcceptRole)
            box.addButton(t("continue_anyway"), QMessageBox.DestructiveRole)
            b_cancel = box.addButton(QMessageBox.Cancel)
            box.exec()
            clicked = box.clickedButton()
            if clicked is b_cancel:
                return
            if clicked is b_close:
                procs.close_apps()
                time.sleep(2)
                still = procs.running_blockers()
                if still:
                    QMessageBox.information(self, t("apps_title"),
                                           t("apps_msg") + "\n\n" + ", ".join(still))

        conn = {k: v for k, v in (("host", self.db_host.text().strip()),
                ("port", self.db_port.text().strip()), ("user", self.db_user.text().strip()),
                ("password", self.db_pass.text())) if v}
        specs = [d for d in dbdump.detect_databases() if d["running"]] if self.cb_databases.isChecked() else []
        cmd = self.db_cmd.text().strip()
        if "=" in cmd:
            n, c = cmd.split("=", 1)
            specs.append({"name": n, "shell": c})
        out_dir = os.path.join(os.path.abspath(self.dest.text()),
                               self.setname.text() or socket.gethostname(), dbdump.DB_DIR)
        func = functools.partial(self._backup_then_db, core_kwargs=kwargs,
                                 specs=specs, conn=conn or None, out_dir=out_dir,
                                 do_vss=self.cb_vss.isChecked())
        self._run(func, {}, self.tr_("tab_backup"))

    # ---------------------------------------------------------- Restore tab
    def _build_restore_tab(self) -> QWidget:
        w = QWidget()
        grid = QGridLayout(w)
        self.lbl_bk_folder = QLabel()
        grid.addWidget(self.lbl_bk_folder, 0, 0)
        self.r_src = QLineEdit()
        grid.addWidget(self.r_src, 0, 1)
        self.btn_pick_bk = QPushButton(); self.btn_pick_bk.clicked.connect(self._pick_backup)
        grid.addWidget(self.btn_pick_bk, 0, 2)

        self.lbl_bk_set = QLabel()
        grid.addWidget(self.lbl_bk_set, 1, 0)
        self.r_set = QComboBox(); self.r_set.setEditable(True)
        grid.addWidget(self.r_set, 1, 1)
        self.btn_load_sets = QPushButton(); self.btn_load_sets.clicked.connect(self._load_sets)
        grid.addWidget(self.btn_load_sets, 1, 2)

        self.lbl_target = QLabel()
        grid.addWidget(self.lbl_target, 2, 0)
        self.r_target = QLineEdit("/")
        grid.addWidget(self.r_target, 2, 1)
        self.btn_target = QPushButton(); self.btn_target.clicked.connect(lambda: self._pick_dir(self.r_target))
        grid.addWidget(self.btn_target, 2, 2)

        self.lbl_r_workers = QLabel()
        grid.addWidget(self.lbl_r_workers, 3, 0)
        self.r_workers = QSpinBox(); self.r_workers.setRange(1, 64)
        self.r_workers.setValue(core.default_workers())
        grid.addWidget(self.r_workers, 3, 1)

        self.cb_meta = QCheckBox(); self.cb_meta.setChecked(True)
        self.cb_r_dry = QCheckBox()
        grid.addWidget(self.cb_meta, 4, 0, 1, 3)
        grid.addWidget(self.cb_r_dry, 5, 0, 1, 3)

        self.btn_start_restore = QPushButton()
        self.btn_start_restore.clicked.connect(self._start_restore)
        grid.addWidget(self.btn_start_restore, 6, 0, 1, 3)
        grid.setRowStretch(7, 1)
        return w

    def _pick_backup(self):
        d = QFileDialog.getExistingDirectory(self, self.tr_("choose"))
        if d:
            self.r_src.setText(d)
            self._load_sets()

    def _load_sets(self):
        sets = core.list_sets(self.r_src.text())
        self.r_set.clear()
        if sets:
            self.r_set.addItems([s["set"] for s in sets])
        else:
            QMessageBox.information(self, self.tr_("sets"), self.tr_("no_sets"))

    def _start_restore(self):
        if not self.r_src.text():
            QMessageBox.warning(self, self.tr_("missing"), self.tr_("backup_folder"))
            return
        if self.r_target.text() == "/" and not self.cb_r_dry.isChecked():
            if QMessageBox.question(self, self.tr_("warning"), self.tr_("confirm_root")) != QMessageBox.Yes:
                return
        kwargs = dict(
            backup_dir=self.r_src.text(), setname=self.r_set.currentText() or None,
            target=self.r_target.text() or "/", workers=self.r_workers.value(),
            reapply_meta=self.cb_meta.isChecked(), dry_run=self.cb_r_dry.isChecked(),
        )
        self._run(core.restore, kwargs, self.tr_("tab_restore"))

    # ------------------------------------------------------ Decommission tab
    def _build_decommission_tab(self) -> QWidget:
        w = QWidget()
        grid = QGridLayout(w)
        r = 0
        self.lbl_d_scope = QLabel()
        grid.addWidget(self.lbl_d_scope, r, 0)
        self.d_scope = QComboBox(); self.d_scope.addItems(["home", "config", "disk"])
        grid.addWidget(self.d_scope, r, 1)
        self.d_sources = QLineEdit(); self.d_sources.setPlaceholderText("/path (scope=config/disk)")
        grid.addWidget(self.d_sources, r, 2); r += 1

        self.lbl_d_dest = QLabel()
        grid.addWidget(self.lbl_d_dest, r, 0)
        self.d_dest = QLineEdit(core.suggested_dest())
        grid.addWidget(self.d_dest, r, 1)
        self.btn_d_dest = QPushButton(); self.btn_d_dest.clicked.connect(lambda: self._pick_dir(self.d_dest))
        grid.addWidget(self.btn_d_dest, r, 2); r += 1

        self.lbl_d_set = QLabel()
        grid.addWidget(self.lbl_d_set, r, 0)
        self.d_set = QLineEdit(); self.d_set.setPlaceholderText(socket.gethostname() + "-decommission")
        grid.addWidget(self.d_set, r, 1, 1, 2); r += 1

        self.lbl_d_enc = QLabel()
        grid.addWidget(self.lbl_d_enc, r, 0)
        self.d_cipher = QComboBox(); self.d_cipher.addItems(["none", "aes256gcm", "chacha20poly1305"])
        grid.addWidget(self.d_cipher, r, 1)
        self.d_pass = QLineEdit(); self.d_pass.setEchoMode(QLineEdit.Password)
        self.d_pass.setPlaceholderText("Password")
        grid.addWidget(self.d_pass, r, 2); r += 1

        self.lbl_d_action = QLabel()
        grid.addWidget(self.lbl_d_action, r, 0)
        self.d_action = QComboBox()
        self.d_action.addItems(["none", "delete", "restore-defaults"])
        grid.addWidget(self.d_action, r, 1)
        self.lbl_d_passes = QLabel()
        grid.addWidget(self.lbl_d_passes, r, 2)
        self.d_passes = QSpinBox(); self.d_passes.setRange(1, 35); self.d_passes.setValue(1)
        grid.addWidget(self.d_passes, r, 3); r += 1

        self.cb_d_secure = QCheckBox()
        grid.addWidget(self.cb_d_secure, r, 0, 1, 4); r += 1

        self.lbl_d_defdir = QLabel()
        grid.addWidget(self.lbl_d_defdir, r, 0)
        self.d_defdir = QLineEdit()
        grid.addWidget(self.d_defdir, r, 1)
        self.btn_d_defdir = QPushButton(); self.btn_d_defdir.clicked.connect(lambda: self._pick_dir(self.d_defdir))
        grid.addWidget(self.btn_d_defdir, r, 2); r += 1

        self.lbl_d_defset = QLabel()
        grid.addWidget(self.lbl_d_defset, r, 0)
        self.d_defset = QLineEdit()
        grid.addWidget(self.d_defset, r, 1)
        self.lbl_d_deftgt = QLabel()
        grid.addWidget(self.lbl_d_deftgt, r, 2)
        self.d_deftgt = QLineEdit("/")
        grid.addWidget(self.d_deftgt, r, 3); r += 1

        self.cb_d_same = QCheckBox()
        self.cb_d_dry = QCheckBox()
        grid.addWidget(self.cb_d_same, r, 0, 1, 2)
        grid.addWidget(self.cb_d_dry, r, 2, 1, 2); r += 1
        self.cb_d_confirm = QCheckBox()
        grid.addWidget(self.cb_d_confirm, r, 0, 1, 4); r += 1

        self.btn_start_decom = QPushButton()
        self.btn_start_decom.clicked.connect(self._start_decommission)
        grid.addWidget(self.btn_start_decom, r, 0, 1, 4); r += 1
        grid.setRowStretch(r, 1)
        return w

    def _start_decommission(self):
        action = self.d_action.currentText()
        secure_wipe = self.cb_d_secure.isChecked()
        dry = self.cb_d_dry.isChecked()
        # Any deletion (delete / restore-defaults / secure-overwrite) is destructive.
        delete_source = action in ("delete", "restore-defaults") or secure_wipe
        if delete_source and not dry and not self.cb_d_confirm.isChecked():
            QMessageBox.warning(self, self.tr_("warning"), self.tr_("need_confirm"))
            return
        cipher = crypto.normalize_cipher(self.d_cipher.currentText())
        if cipher != crypto.CIPHER_NONE:
            if not self.d_pass.text():
                QMessageBox.warning(self, self.tr_("missing"), self.tr_("need_password"))
                return
            try:
                crypto.ensure_available()
            except crypto.CryptoUnavailable as e:
                QMessageBox.critical(self, self.tr_("error"), str(e))
                return
        scope = self.d_scope.currentText()
        explicit = [s.strip() for s in self.d_sources.text().split(";") if s.strip()]
        try:
            resolved = core.scope_sources(scope, explicit)
        except ValueError as e:
            QMessageBox.warning(self, self.tr_("missing"), str(e))
            return
        if not self.d_dest.text():
            QMessageBox.warning(self, self.tr_("missing"), self.tr_("need_dest"))
            return
        excludes = core.disk_excludes() if scope == "disk" else []
        defaults = None
        def_pw = self.d_pass.text() if cipher != crypto.CIPHER_NONE else None
        if action == "restore-defaults":
            if not self.d_defdir.text():
                QMessageBox.warning(self, self.tr_("missing"), self.tr_("defaults_dir"))
                return
            defaults = (self.d_defdir.text(), self.d_defset.text() or None,
                        self.d_deftgt.text() or "/")
        kwargs = dict(
            sources=resolved, dest=self.d_dest.text(),
            setname=self.d_set.text() or None, workers=core.default_workers(),
            extra_excludes=excludes, scope=scope, delete_source=delete_source,
            allow_same_device=self.cb_d_same.isChecked(), cipher=cipher,
            passphrase=(self.d_pass.text() if cipher != crypto.CIPHER_NONE else None),
            secure_wipe=secure_wipe, wipe_passes=self.d_passes.value(),
            dry_run=dry,
        )
        func = functools.partial(_decommission_job, defaults=defaults, def_pw=def_pw)
        self._run(func, kwargs, self.tr_("tab_decommission"))

    # ------------------------------------------------------------- Clone tab
    def _build_clone_tab(self) -> QWidget:
        w = QWidget()
        grid = QGridLayout(w)
        self.lbl_c_tool = QLabel()
        grid.addWidget(self.lbl_c_tool, 0, 0)
        self.c_tool = QComboBox()
        grid.addWidget(self.c_tool, 0, 1)
        self.btn_c_detect = QPushButton(); self.btn_c_detect.clicked.connect(self._detect_tools)
        grid.addWidget(self.btn_c_detect, 0, 2)

        self.lbl_c_src = QLabel()
        grid.addWidget(self.lbl_c_src, 1, 0)
        self.c_src = QLineEdit(); self.c_src.setPlaceholderText("/dev/disk2 or image.raw")
        grid.addWidget(self.c_src, 1, 1, 1, 2)

        self.lbl_c_tgt = QLabel()
        grid.addWidget(self.lbl_c_tgt, 2, 0)
        self.c_tgt = QLineEdit(); self.c_tgt.setPlaceholderText("/Volumes/Stick/image.raw")
        grid.addWidget(self.c_tgt, 2, 1, 1, 2)

        self.cb_c_dry = QCheckBox()
        grid.addWidget(self.cb_c_dry, 3, 0, 1, 3)
        self.cb_c_confirm = QCheckBox()
        grid.addWidget(self.cb_c_confirm, 4, 0, 1, 3)

        self.btn_start_clone = QPushButton()
        self.btn_start_clone.clicked.connect(self._start_clone)
        grid.addWidget(self.btn_start_clone, 5, 0, 1, 3)
        grid.setRowStretch(6, 1)
        self._detect_tools()
        return w

    def _detect_tools(self):
        tools = reset.detect_clone_tools()
        self.c_tool.clear()
        self.c_tool.addItems([t["name"] for t in tools if t["available"]])
        summary = "  ".join(f"[{'✓' if t['available'] else '—'}] {t['name']}" for t in tools)
        if hasattr(self, "status"):  # not yet created during initial tab build
            self.status.setText(summary)

    def _start_clone(self):
        tool = self.c_tool.currentText()
        src, tgt = self.c_src.text(), self.c_tgt.text()
        dry = self.cb_c_dry.isChecked()
        if not tool or not src or not tgt:
            QMessageBox.warning(self, self.tr_("missing"), self.tr_("need_target"))
            return
        if not dry and not self.cb_c_confirm.isChecked():
            QMessageBox.warning(self, self.tr_("warning"), self.tr_("need_confirm"))
            return
        try:
            argv = reset.build_clone_command(tool, src, tgt)
        except ValueError as e:
            QMessageBox.warning(self, self.tr_("error"), str(e))
            return
        func = functools.partial(_clone_job, argv=argv, dry=dry)
        self._run(func, {}, self.tr_("tab_clone"))

    # --------------------------------------------------------------- i18n
    def _change_language(self, index):
        code = self.lang_combo.itemData(index)
        self.tr_.set_language(code)
        direction = Qt.RightToLeft if self.tr_.is_rtl() else Qt.LeftToRight
        QApplication.instance().setLayoutDirection(direction)
        self.retranslate()

    def retranslate(self):
        t = self.tr_
        self.setWindowTitle(f"{t('app_title')} {__version__}")
        self.lbl_language.setText(t("language"))
        self.tabs.setTabText(0, t("tab_backup"))
        self.tabs.setTabText(1, t("tab_restore"))
        self.tabs.setTabText(2, t("tab_decommission"))
        self.tabs.setTabText(3, t("tab_clone"))
        # backup tab
        self.gb_sources.setTitle(t("sources"))
        self.btn_add_dir.setText(t("add_folder"))
        self.btn_add_file.setText(t("add_file"))
        self.btn_remove.setText(t("remove"))
        self.btn_auto.setText(t("auto_sources"))
        self.lbl_dest.setText(t("dest"))
        self.btn_dest.setText(t("choose"))
        self.lbl_set.setText(t("set_name"))
        self.lbl_set_hint.setText(t("per_system_hint"))
        self.lbl_workers.setText(t("workers"))
        self.lbl_excl.setText(t("excludes"))
        self.lbl_extra.setText(t("extra_paths"))
        self.lbl_uid.setText(t("uid_label"))
        self.gb_db.setTitle(t("db_group"))
        self.cb_databases.setText(t("db_dump"))
        self.lbl_db_cmd.setText(t("db_cmd_label"))
        self.cb_checksum.setText(t("opt_checksum"))
        self.cb_delete.setText(t("opt_delete"))
        self.cb_system.setText(t("opt_system"))
        self.cb_dry.setText(t("opt_dryrun"))
        self.cb_vss.setText(t("opt_vss"))
        self.btn_start_backup.setText(t("start_backup"))
        # restore tab
        self.lbl_bk_folder.setText(t("backup_folder"))
        self.btn_pick_bk.setText(t("choose"))
        self.lbl_bk_set.setText(t("backup_set"))
        self.btn_load_sets.setText(t("load_sets"))
        self.lbl_target.setText(t("target_root"))
        self.btn_target.setText(t("choose"))
        self.lbl_r_workers.setText(t("workers"))
        self.cb_meta.setText(t("reapply_meta"))
        self.cb_r_dry.setText(t("opt_dryrun"))
        self.btn_start_restore.setText(t("start_restore"))
        # decommission tab
        self.lbl_d_scope.setText(t("scope"))
        self.lbl_d_dest.setText(t("dest"))
        self.btn_d_dest.setText(t("choose"))
        self.lbl_d_set.setText(t("set_name"))
        self.lbl_d_enc.setText(t("encryption"))
        self.lbl_d_action.setText(t("reset_action"))
        self.cb_d_secure.setText(t("secure_wipe"))
        self.lbl_d_passes.setText(t("wipe_passes"))
        self.lbl_d_defdir.setText(t("defaults_dir"))
        self.btn_d_defdir.setText(t("choose"))
        self.lbl_d_defset.setText(t("defaults_set"))
        self.lbl_d_deftgt.setText(t("defaults_target"))
        self.cb_d_same.setText(t("allow_same_device"))
        self.cb_d_dry.setText(t("opt_dryrun"))
        self.cb_d_confirm.setText(t("confirm_erase"))
        self.btn_start_decom.setText(t("start_decommission"))
        # clone tab
        self.lbl_c_tool.setText(t("clone_tool"))
        self.btn_c_detect.setText(t("list_tools"))
        self.lbl_c_src.setText(t("clone_source"))
        self.lbl_c_tgt.setText(t("clone_target"))
        self.cb_c_dry.setText(t("opt_dryrun"))
        self.cb_c_confirm.setText(t("confirm_clone"))
        self.btn_start_clone.setText(t("start_clone"))
        if not self.status.text():
            self.status.setText(t("ready"))

    # -------------------------------------------------------------- runner
    def _session_log_path(self):
        import os
        import sys
        import time
        import tempfile
        # Prefer the directory the app runs from (portable: log lands next to the
        # app/launcher); fall back to the home dir, then temp, if that's read-only.
        candidates = []
        try:
            if getattr(sys, "frozen", False):
                candidates.append(os.path.dirname(os.path.abspath(sys.executable)))
            elif sys.argv and sys.argv[0]:
                candidates.append(os.path.dirname(os.path.abspath(sys.argv[0])))
        except Exception:
            pass
        candidates.append(os.environ.get("USERPROFILE") or os.path.expanduser("~"))
        candidates.append(tempfile.gettempdir())
        stamp = time.strftime("%Y%m%d-%H%M%S")
        for base in candidates:
            if not base:
                continue
            d = os.path.join(base, "backuptool-logs")
            try:
                os.makedirs(d, exist_ok=True)
                test = os.path.join(d, ".wtest")
                with open(test, "w"):
                    pass
                os.remove(test)
                return os.path.join(d, "backuptool-gui-" + stamp + ".log")
            except OSError:
                continue
        return os.path.join(tempfile.gettempdir(), "backuptool-gui-" + stamp + ".log")

    def _close_logfh(self):
        fh = getattr(self, "_logfh", None)
        if fh:
            try:
                fh.close()
            except OSError:
                pass
            self._logfh = None

    def _run(self, func, kwargs, label):
        if self._thread is not None and self._thread.isRunning():
            QMessageBox.information(self, label, self.tr_("busy"))
            return
        self.logbox.clear()
        self.progress.setValue(0)
        self.status.setText(f"{label} {self.tr_('running')}")

        # Open a timestamped session log file and announce its path.
        self._close_logfh()
        try:
            self._logpath = self._session_log_path()
            self._logfh = open(self._logpath, "a", encoding="utf-8")
            header = f"Log file: {self._logpath}"
            self.logbox.appendPlainText(header)
            self._logfh.write(header + "\n")
            self._logfh.flush()
        except OSError:
            self._logfh = None

        self._thread = QThread()
        self._worker = Worker(func, kwargs, label)
        self._worker.moveToThread(self._thread)
        self._thread.started.connect(self._worker.run)
        self._worker.log.connect(self._on_log)
        self._worker.progress.connect(self._on_progress)
        self._worker.finished.connect(self._on_finished)
        self._worker.failed.connect(self._on_failed)
        self._worker.finished.connect(self._thread.quit)
        self._worker.failed.connect(self._thread.quit)
        self._thread.start()

    @Slot(str)
    def _on_log(self, msg):
        self.logbox.appendPlainText(msg)
        fh = getattr(self, "_logfh", None)
        if fh:
            try:
                fh.write(msg + "\n")
                fh.flush()
            except OSError:
                pass

    @Slot(int, int)
    def _on_progress(self, done, total):
        self.progress.setMaximum(max(1, total))
        self.progress.setValue(done)
        self.status.setText(f"{done}/{total}")

    @Slot(str)
    def _on_finished(self, label):
        self.status.setText(f"{label} {self.tr_('done_suffix')}")
        self._close_logfh()

    @Slot(str)
    def _on_failed(self, err):
        self.status.setText(self.tr_("error"))
        self._close_logfh()
        QMessageBox.critical(self, self.tr_("error"), err)


def main():
    app = QApplication(sys.argv)
    win = MainWindow()
    win.show()
    sys.exit(app.exec())


if __name__ == "__main__":
    main()
