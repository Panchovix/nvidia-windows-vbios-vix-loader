"""vbios-loader-gui: a window to pick a GPU and a VBIOS file, then load or remove the override.

Widgets only; the state, checks and texts are in controller.py, the registry work in operations.py.
VBIOS_LOADER_GUI_SMOKE=1 opens the window, reads the status, switches to the other language and theme, and
closes it after a few seconds.
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

from . import __version__
from .cli import windows_platform
from .controller import LANGUAGES, THEMES, Controller, resolve_theme, system_language
from .operations import LoaderError

PROG = 'vbios-loader-gui'
SMOKE_ENV = 'VBIOS_LOADER_GUI_SMOKE'
SMOKE_SECONDS = 20
COLUMNS = (('col_index', 40), ('col_name', 230), ('col_pci', 90), ('col_location', 80), ('col_driver', 150),
           ('col_override', 300))
# Light keeps the native ttk theme; dark draws ttk's clam theme with these colors.
PALETTES = {
    'light': dict(ok='#176317', bad='#9b1c1c', banner_bg='#fde2e1', banner_fg='#7a1010', text_bg='#ffffff',
                  text_fg='#000000', select='#0078d7', select_fg='#ffffff'),
    'dark': dict(ok='#7fd88f', bad='#ff8a80', banner_bg='#4a1f1f', banner_fg='#ffd8d4', text_bg='#1b1b1b',
                 text_fg='#e8e8e8', select='#1f5fa8', select_fg='#ffffff', bg='#202020', field='#2b2b2b',
                 fg='#e8e8e8', border='#454545', button='#333333', active='#404040', pressed='#4a4a4a',
                 disabled='#7a7a7a'),
}
DWMWA_USE_IMMERSIVE_DARK_MODE = (20, 19)     # Windows 10 20H1 and later; 19 before


def style_dark(style, p):
    style.theme_use('clam')
    style.configure('.', background=p['bg'], foreground=p['fg'], fieldbackground=p['field'], bordercolor=p['border'],
                    darkcolor=p['bg'], lightcolor=p['bg'], troughcolor=p['bg'], selectbackground=p['select'],
                    selectforeground=p['select_fg'], insertcolor=p['fg'], focuscolor=p['border'])
    style.map('.', foreground=[('disabled', p['disabled'])], background=[('disabled', p['bg'])])
    style.configure('TButton', background=p['button'], padding=(10, 3))
    style.map('TButton', background=[('disabled', p['bg']), ('pressed', p['pressed']), ('active', p['active'])])
    style.configure('TCheckbutton', indicatorbackground=p['field'], indicatorforeground=p['fg'])
    style.map('TCheckbutton', background=[('active', p['bg'])],
              indicatorbackground=[('disabled', p['bg']), ('selected', p['select'])])
    style.map('TEntry', fieldbackground=[('readonly', p['field'])], foreground=[('readonly', p['fg'])])
    style.configure('Treeview', background=p['field'], fieldbackground=p['field'], foreground=p['fg'])
    style.map('Treeview', background=[('selected', p['select'])], foreground=[('selected', p['select_fg'])])
    style.configure('Treeview.Heading', background=p['button'], foreground=p['fg'])
    style.map('Treeview.Heading', background=[('active', p['active'])])
    style.configure('TLabelframe.Label', foreground=p['fg'])
    style.configure('TScrollbar', background=p['button'], arrowcolor=p['fg'])
    style.configure('TMenubutton', background=p['button'], arrowcolor=p['fg'], padding=(8, 2))
    style.map('TMenubutton', background=[('pressed', p['pressed']), ('active', p['active'])])
    style.map('TScrollbar', background=[('active', p['active'])])


class Window:
    def __init__(self, root, controller):
        self.root, self.c = root, controller
        self.results = queue.Queue()
        self.failed = False
        root.minsize(780, 540)
        root.columnconfigure(0, weight=1)
        root.rowconfigure(2, weight=1)
        root.rowconfigure(5, weight=2)
        root.protocol('WM_DELETE_WINDOW', self.close)
        root.report_callback_exception = self.callback_error
        self.style = ttk.Style(root)
        self.native_theme = self.style.theme_use()
        self.language = tk.StringVar(value=controller.language)
        self.theme = tk.StringVar(value=controller.theme)
        self.apply_theme()
        self.build()

    def apply_theme(self):
        self.palette = p = PALETTES[resolve_theme(self.c.theme)]
        if 'bg' in p:
            style_dark(self.style, p)
        else:
            self.style.theme_use(self.native_theme)
        self.style.configure('Treeview', rowheight=font.nametofont('TkDefaultFont').metrics('linespace') + 6)
        background = p.get('bg') or self.style.lookup('.', 'background')
        if background:
            self.root.configure(bg=background)
        self.root.after(50, self.dark_title_bar)

    def dark_title_bar(self):
        """Ask Windows to draw this window's title bar dark or light, like the rest of the window."""
        if os.name != 'nt':
            return
        try:
            hwnd = ctypes.windll.user32.GetParent(self.root.winfo_id())
            value = ctypes.c_int(int('bg' in self.palette))
            for attribute in DWMWA_USE_IMMERSIVE_DARK_MODE:
                if ctypes.windll.dwmapi.DwmSetWindowAttribute(hwnd, attribute, ctypes.byref(value),
                                                              ctypes.sizeof(value)) == 0:
                    break
            # Redraw the frame now: SWP_NOSIZE | SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED.
            ctypes.windll.user32.SetWindowPos(hwnd, None, 0, 0, 0, 0, 0x1 | 0x2 | 0x4 | 0x10 | 0x20)
        except (AttributeError, OSError, tk.TclError):
            pass

    def build(self):
        """Create the widgets in the controller's language (again after a language change)."""
        root, controller = self.root, self.c
        self.t = t = controller.text
        root.title(f"{t['title']} {__version__}")
        # Menu buttons instead of a menu bar: the Windows menu bar cannot be drawn dark.
        p = self.palette
        colors = (dict(bg=p['field'], fg=p['fg'], activebackground=p['select'], activeforeground=p['select_fg'],
                       selectcolor=p['fg'], relief='flat') if 'bg' in p else {})
        bar = ttk.Frame(root, padding=(8, 6, 8, 0))
        bar.grid(row=0, column=0, sticky='ew')
        choices = ((t['language'], [(name, code) for code, name in LANGUAGES], self.language, self.set_language),
                   (t['theme'], [(t['theme_' + mode], mode) for mode in THEMES], self.theme, self.set_theme))
        for column, (title, items, variable, command) in enumerate(choices):
            button = ttk.Menubutton(bar, text=title)
            menu = tk.Menu(button, tearoff=False, **colors)
            for name, value in items:
                menu.add_radiobutton(label=name, value=value, variable=variable,
                                     command=lambda value=value, command=command: command(value))
            button['menu'] = menu
            button.grid(row=0, column=column, padx=(0, 6))

        self.banner = tk.Label(root, text=t['not_admin'], bg=p['banner_bg'], fg=p['banner_fg'], anchor='w',
                               justify='left', padx=10, pady=6, wraplength=740)
        self.banner.grid(row=1, column=0, sticky='ew', padx=8, pady=(8, 0))

        gpus = ttk.LabelFrame(root, text=t['gpus'], padding=6)
        gpus.grid(row=2, column=0, sticky='nsew', padx=8, pady=(8, 0))
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
        files.grid(row=3, column=0, sticky='ew', padx=8, pady=(8, 0))
        files.columnconfigure(1, weight=1)
        self.file_button = ttk.Button(files, text=t['file_button'], command=self.choose_file)
        self.file_button.grid(row=0, column=0, sticky='w')
        self.path = tk.StringVar()
        ttk.Entry(files, textvariable=self.path, state='readonly').grid(row=0, column=1, sticky='ew', padx=(6, 0))
        self.validation = ttk.Label(files, wraplength=740, justify='left')
        self.validation.grid(row=1, column=0, columnspan=2, sticky='w', pady=(6, 0))

        actions = ttk.Frame(root, padding=(8, 8, 8, 0))
        actions.grid(row=4, column=0, sticky='ew')
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
        messages.grid(row=5, column=0, sticky='nsew', padx=8, pady=8)
        messages.columnconfigure(0, weight=1)
        messages.rowconfigure(0, weight=1)
        self.log_text = tk.Text(messages, height=8, wrap='word', state='disabled', bg=p['text_bg'], fg=p['text_fg'],
                                insertbackground=p['text_fg'], selectbackground=p['select'],
                                selectforeground=p['select_fg'], relief='flat', highlightthickness=0)
        log_scroll = ttk.Scrollbar(messages, orient='vertical', command=self.log_text.yview)
        self.log_text.configure(yscrollcommand=log_scroll.set)
        self.log_text.grid(row=0, column=0, sticky='nsew')
        log_scroll.grid(row=0, column=1, sticky='ns')

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
        self.fill()

    def fill(self):
        self.tree.delete(*self.tree.get_children())
        for row in self.c.rows():
            self.tree.insert('', 'end', iid=str(row[0]), values=row)
        if self.c.gpu is not None:
            self.tree.selection_set(str(self.c.gpu['index']))
        self.update_controls()

    def set_language(self, language):
        if self.c.busy:                  # the running action finishes in the current language
            self.language.set(self.c.language)
            return
        self.c.set_language(language)
        self.rebuild()

    def set_theme(self, theme):
        if self.c.busy:
            self.theme.set(self.c.theme)
            return
        self.c.theme = theme
        self.apply_theme()
        self.rebuild()

    def rebuild(self):
        messages = self.log_text.get('1.0', 'end-1c')
        for child in self.root.winfo_children():
            child.destroy()
        self.build()
        if messages:
            self.log([messages])
        self.fill()

    def update_controls(self):
        c = self.c
        if c.admin:
            self.banner.grid_remove()
        else:
            self.banner.grid()
        self.path.set(c.path or '')
        ok, text = c.validation_text()
        self.validation.configure(text=text, foreground=self.palette['ok' if ok else 'bad'])
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
    parser.add_argument('--theme', choices=THEMES, default='auto',
                        help='light or dark (default auto: the Windows app mode)')
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
    window = Window(root, Controller(platform, args.lang or system_language(), args.theme))
    window.refresh()
    if smoke:
        root.attributes('-topmost', True)
        # The window is rebuilt in the other language and the other theme, so the CI screenshot shows both.
        other = next(code for code, _ in LANGUAGES if code != window.c.language)
        root.after(3000, lambda: window.set_language(other))
        contrast = 'light' if resolve_theme(window.c.theme) == 'dark' else 'dark'
        root.after(5000, lambda: window.set_theme(contrast))
        root.after(SMOKE_SECONDS * 1000, root.destroy)
    root.mainloop()
    return 1 if smoke and window.failed else 0


if __name__ == '__main__':
    raise SystemExit(main())
