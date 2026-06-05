# SPDX-License-Identifier: GPL-3.0-or-later
"""Qt6 (PySide6) GUI – cross-platform (Linux/macOS/Windows), multilingual.

The core engine (core.py) is GUI-independent. UI strings are loaded from JSON
catalogs via the i18n module, with a language selector (EN/DE/FA and any extra
catalog dropped into lang/). Right-to-left languages (e.g. Persian) switch the
layout direction automatically.

Requires:  pip install PySide6
"""
from __future__ import annotations

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

from . import __version__, core, discover
from .i18n import Translator, available


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
        self._is_root = hasattr(os, "geteuid") and os.geteuid() == 0
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
        _auto_dest = discover.default_destination()
        if _auto_dest:
            self.dest.setText(_auto_dest)   # USB / external / network if found
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
        lay.addLayout(grid)

        self.cb_checksum = QCheckBox()
        self.cb_delete = QCheckBox()
        self.cb_system = QCheckBox()
        self.cb_dry = QCheckBox()
        lay.addWidget(self.cb_checksum); lay.addWidget(self.cb_delete)
        lay.addWidget(self.cb_system); lay.addWidget(self.cb_dry)

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

    def _start_backup(self):
        sources = [self.sources.item(i).text() for i in range(self.sources.count())]
        if not sources:
            QMessageBox.warning(self, self.tr_("missing"), self.tr_("need_source"))
            return
        if not self.dest.text():
            QMessageBox.warning(self, self.tr_("missing"), self.tr_("need_dest"))
            return
        excludes = [x.strip() for x in self.excl.text().split(",") if x.strip()]
        kwargs = dict(
            sources=sources, dest=self.dest.text(),
            setname=self.setname.text() or None, workers=self.workers.value(),
            use_checksum=self.cb_checksum.isChecked(), extra_excludes=excludes,
            prune=self.cb_delete.isChecked(), dry_run=self.cb_dry.isChecked(),
            include_system=self.cb_system.isChecked(),
        )
        self._run(core.backup, kwargs, self.tr_("tab_backup"))

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
        self.cb_checksum.setText(t("opt_checksum"))
        self.cb_delete.setText(t("opt_delete"))
        self.cb_system.setText(t("opt_system"))
        self.cb_dry.setText(t("opt_dryrun"))
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
        # Under root the native file dialog often cannot open (no user portal);
        # hint the user to type paths into the fields instead.
        if self._is_root:
            self.status.setText(t("root_hint"))
        elif not self.status.text():
            self.status.setText(t("ready"))

    # -------------------------------------------------------------- runner
    def _run(self, func, kwargs, label):
        if self._thread is not None and self._thread.isRunning():
            QMessageBox.information(self, label, self.tr_("busy"))
            return
        self.logbox.clear()
        self.progress.setValue(0)
        self.status.setText(f"{label} {self.tr_('running')}")

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

    @Slot(int, int)
    def _on_progress(self, done, total):
        self.progress.setMaximum(max(1, total))
        self.progress.setValue(done)
        self.status.setText(f"{done}/{total}")

    @Slot(str)
    def _on_finished(self, label):
        self.status.setText(f"{label} {self.tr_('done_suffix')}")

    @Slot(str)
    def _on_failed(self, err):
        self.status.setText(self.tr_("error"))
        QMessageBox.critical(self, self.tr_("error"), err)


def main():
    app = QApplication(sys.argv)
    win = MainWindow()
    win.show()
    sys.exit(app.exec())


if __name__ == "__main__":
    main()
