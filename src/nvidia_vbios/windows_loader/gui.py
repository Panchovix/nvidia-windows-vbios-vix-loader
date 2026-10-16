"""vbios-loader-gui: a window to pick a GPU and a VBIOS file, then load or remove the override.

Widgets only; the state, checks and texts are in controller.py, the registry work in operations.py.
VBIOS_LOADER_GUI_SMOKE=1 opens the window, reads the status, and closes it after a few seconds.
"""
from __future__ import annotations
import argparse
import ctypes
import os
import queue
import sys
import threading
import tkinter as tk
import traceback
from tkinter import filedialog, font, messagebox, ttk
from tkinter.scrolledtext import ScrolledText

from . import __version__
from .cli import windows_platform
from .controller import Controller, system_language
from .operations import LoaderError

PROG = 'vbios-loader-gui'
SMOKE_ENV = 'VBIOS_LOADER_GUI_SMOKE'
SMOKE_SECONDS = 20
COLUMNS = (('col_index', 40), ('col_name', 230), ('col_pci', 90), ('col_location', 80), ('col_driver', 150),
           ('col_override', 300))


class Window:
    def __init__(self, root, controller):
        self.root, self.c, self.t = root, controller, controller.text
        self.results = queue.Queue()
        self.failed = False
        t = self.t
        root.title(f"{t['title']} {__version__}")
        root.minsize(780, 540)
        root.columnconfigure(0, weight=1)
        root.rowconfigure(1, weight=1)
        root.rowconfigure(4, weight=2)
        root.protocol('WM_DELETE_WINDOW', self.close)
        root.report_callback_exception = self.callback_error
        ttk.Style(root).configure('Treeview', rowheight=font.nametofont('TkDefaultFont').metrics('linespace') + 6)

        self.banner = tk.Label(root, text=t['not_admin'], bg='#fde2e1', fg='#7a1010', anchor='w', justify='left',
                               padx=10, pady=6, wraplength=740)
        self.banner.grid(row=0, column=0, sticky='ew', padx=8, pady=(8, 0))

        gpus = ttk.LabelFrame(root, text=t['gpus'], padding=6)
        gpus.grid(row=1, column=0, sticky='nsew', padx=8, pady=(8, 0))
        gpus.columnconfigure(0, weight=1)
        gpus.rowconfigure(0, weight=1)
        self.tree = ttk.Treeview(gpus, columns=[key for key, _ in COLUMNS], show='headings', selectmode='browse',
                                 height=4)
        for key, width in COLUMNS:
            self.tree.heading(key, text=t[key], anchor='w')
            self.tree.column(key, width=width, minwidth=30, stretch=key in ('col_name', 'col_override'))
        scroll = ttk.Scrollbar(gpus, orient='vertical', command=self.tree.yview)
        self.tree.configure(yscrollcommand=scroll.set)
        self.tree.grid(row=0, column=0, sticky='nsew')
        scroll.grid(row=0, column=1, sticky='ns')
        self.tree.bind('<<TreeviewSelect>>', self.on_select)
        self.refresh_button = ttk.Button(gpus, text=t['refresh'], command=self.refresh)
        self.refresh_button.grid(row=1, column=0, columnspan=2, sticky='e', pady=(6, 0))

        files = ttk.LabelFrame(root, text=t['file'], padding=6)
        files.grid(row=2, column=0, sticky='ew', padx=8, pady=(8, 0))
        files.columnconfigure(1, weight=1)
        self.file_button = ttk.Button(files, text=t['file_button'], command=self.choose_file)
        self.file_button.grid(row=0, column=0, sticky='w')
        self.path = tk.StringVar()
        ttk.Entry(files, textvariable=self.path, state='readonly').grid(row=0, column=1, sticky='ew', padx=(6, 0))
        self.validation = ttk.Label(files, wraplength=740, justify='left')
        self.validation.grid(row=1, column=0, columnspan=2, sticky='w', pady=(6, 0))

        actions = ttk.Frame(root, padding=(8, 8, 8, 0))
        actions.grid(row=3, column=0, sticky='ew')
        actions.columnconfigure(1, weight=1)
        self.restart = tk.BooleanVar(value=controller.restart)
        self.restart_box = ttk.Checkbutton(actions, text=t['restart'], variable=self.restart,
                                           command=lambda: setattr(self.c, 'restart', self.restart.get()))
        self.restart_box.grid(row=0, column=0, sticky='w')
        self.working = ttk.Label(actions, text='')
        self.working.grid(row=0, column=1, sticky='w', padx=12)
        self.load_button = ttk.Button(actions, text=t['load'], command=self.load)
        self.load_button.grid(row=0, column=2, padx=(0, 6))
        self.remove_button = ttk.Button(actions, text=t['remove'], command=self.remove)
        self.remove_button.grid(row=0, column=3)

        messages = ttk.LabelFrame(root, text=t['messages'], padding=6)
        messages.grid(row=4, column=0, sticky='nsew', padx=8, pady=8)
        messages.columnconfigure(0, weight=1)
        messages.rowconfigure(0, weight=1)
        self.log_text = ScrolledText(messages, height=8, wrap='word', state='disabled')
        self.log_text.grid(row=0, column=0, sticky='nsew')

    def log(self, lines):
        if not lines:
            return
        self.log_text.configure(state='normal')
        self.log_text.insert('end', '\n'.join(lines) + '\n')
        self.log_text.see('end')
        self.log_text.configure(state='disabled')

    def callback_error(self, kind, error, trace):
        self.failed = True
        if sys.stderr is not None:   # reaches the CI log when the output is redirected
            traceback.print_exception(kind, error, trace, file=sys.stderr)
        self.log([self.t['unexpected'].format(message=f'{kind.__name__}: {error}')])

    def refresh(self):
        self.log(self.c.refresh())
        self.tree.delete(*self.tree.get_children())
        for row in self.c.rows():
            self.tree.insert('', 'end', iid=str(row[0]), values=row)
        if self.c.gpu is not None:
            self.tree.selection_set(str(self.c.gpu['index']))
        self.update_controls()

    def update_controls(self):
        c = self.c
        if c.admin:
            self.banner.grid_remove()
        else:
            self.banner.grid()
        self.path.set(c.path or '')
        ok, text = c.validation_text()
        self.validation.configure(text=text, foreground='#176317' if ok else '#9b1c1c')
        idle = ['!disabled'] if not c.busy else ['disabled']
        for widget in (self.refresh_button, self.file_button, self.restart_box):
            widget.state(idle)
        self.load_button.state(['!disabled'] if c.can_load() else ['disabled'])
        self.remove_button.state(['!disabled'] if c.can_remove() else ['disabled'])
        self.working.configure(text=self.t['working'] if c.busy else '')

    def on_select(self, _event=None):
        selection = self.tree.selection()
        if selection and not self.c.busy:
            self.c.select(int(selection[0]))
            self.update_controls()

    def choose_file(self):
        path = filedialog.askopenfilename(parent=self.root, title=self.t['file'],
                                          filetypes=[(self.t['file_types'], '*.rom *.bin'),
                                                     (self.t['all_files'], '*.*')])
        if path:
            self.c.choose_file(path)
            self.update_controls()

    def load(self):
        question = self.c.confirm_load()     # checks again now: the registry may have changed since the file was chosen
        self.update_controls()
        if question and messagebox.askokcancel(self.t['confirm_load_title'], question, icon='warning',
                                               parent=self.root):
            self.start('load')

    def remove(self):
        if not self.c.can_remove():
            return
        question, error = self.c.confirm_remove()
        if error:
            self.log([error])
        elif messagebox.askokcancel(self.t['confirm_remove_title'], question, icon='warning', parent=self.root):
            self.start('unload')

    def start(self, action):
        self.c.busy = True
        self.update_controls()
        threading.Thread(target=lambda: self.results.put(self.c.run(action)), daemon=True).start()
        self.root.after(200, self.poll)

    def poll(self):
        try:
            ok, lines = self.results.get_nowait()
        except queue.Empty:
            self.root.after(200, self.poll)
            return
        self.c.busy = False
        self.log(lines)
        self.refresh()
        (messagebox.showinfo if ok else messagebox.showerror)(self.t['title'], '\n\n'.join(lines), parent=self.root)

    def close(self):
        if not self.c.busy:       # a load or unload in progress finishes first
            self.root.destroy()


def _dpi_aware():
    try:
        ctypes.windll.shcore.SetProcessDpiAwareness(1)
    except (AttributeError, OSError):
        pass


def main(argv=None, platform=None):
    parser = argparse.ArgumentParser(prog=PROG, description=__doc__.split('\n')[0])
    parser.add_argument('--version', action='version', version=f'{PROG} {__version__}')
    parser.add_argument('--lang', choices=('en', 'es'), help='language (default: the Windows display language)')
    args = parser.parse_args(argv)
    try:
        platform = platform or windows_platform()
    except LoaderError as exc:
        print(f'{PROG}: {exc}', file=sys.stderr)
        return 1
    smoke = os.environ.get(SMOKE_ENV) == '1'
    if os.name == 'nt':
        _dpi_aware()
    root = tk.Tk()
    window = Window(root, Controller(platform, args.lang or system_language()))
    window.refresh()
    if smoke:
        root.attributes('-topmost', True)
        root.after(SMOKE_SECONDS * 1000, root.destroy)
    root.mainloop()
    return 1 if smoke and window.failed else 0


if __name__ == '__main__':
    raise SystemExit(main())
