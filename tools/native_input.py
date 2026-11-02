"""Native macOS input for desktop harnesses. Never sends input through horizon-ctl.

Requires Accessibility permission for the invoking process. Only use while the person
at the desktop has agreed to let the harness take focus and move the pointer.
"""
import ctypes as c
import plistlib
import sys
import time


class Point(c.Structure):
    _fields_ = [('x', c.c_double), ('y', c.c_double)]


class MacInput:
    KEYS = {'w': 13, 'a': 0, 'escape': 53, 'e': 14}

    def __init__(self, pid):
        if sys.platform != 'darwin':
            raise RuntimeError('native input harness currently supports macOS')
        self.pid = pid
        self.cg = c.CDLL('/System/Library/Frameworks/CoreGraphics.framework/CoreGraphics')
        self.cf = c.CDLL('/System/Library/Frameworks/CoreFoundation.framework/CoreFoundation')
        self.ax = c.CDLL('/System/Library/Frameworks/ApplicationServices.framework/ApplicationServices')
        def bind(lib, name, result, *args):
            fn = getattr(lib, name)
            fn.restype, fn.argtypes = result, list(args)
        bind(self.ax, 'AXIsProcessTrusted', c.c_bool)
        if not self.ax.AXIsProcessTrusted():
            raise RuntimeError('native input requires macOS Accessibility permission')
        bind(self.ax, 'AXUIElementCreateApplication', c.c_void_p, c.c_int)
        bind(self.ax, 'AXUIElementSetAttributeValue', c.c_int, c.c_void_p, c.c_void_p, c.c_void_p)
        bind(self.ax, 'AXUIElementCopyAttributeValue', c.c_int, c.c_void_p, c.c_void_p, c.POINTER(c.c_void_p))
        bind(self.ax, 'AXUIElementPerformAction', c.c_int, c.c_void_p, c.c_void_p)
        bind(self.cf, 'CFArrayGetCount', c.c_long, c.c_void_p)
        bind(self.cf, 'CFArrayGetValueAtIndex', c.c_void_p, c.c_void_p, c.c_long)
        bind(self.cf, 'CFStringGetCString', c.c_bool, c.c_void_p, c.c_void_p, c.c_long, c.c_uint32)
        bind(self.cf, 'CFStringCreateWithCString', c.c_void_p, c.c_void_p, c.c_char_p, c.c_uint32)
        bind(self.cf, 'CFRelease', None, c.c_void_p)
        bind(self.cg, 'CGWindowListCopyWindowInfo', c.c_void_p, c.c_uint32, c.c_uint32)
        bind(self.cf, 'CFPropertyListCreateData', c.c_void_p, c.c_void_p, c.c_void_p, c.c_long, c.c_ulong, c.c_void_p)
        bind(self.cf, 'CFDataGetLength', c.c_long, c.c_void_p)
        bind(self.cf, 'CFDataGetBytePtr', c.c_void_p, c.c_void_p)
        bind(self.cg, 'CGEventCreateKeyboardEvent', c.c_void_p, c.c_void_p, c.c_uint16, c.c_bool)
        bind(self.cg, 'CGEventCreateMouseEvent', c.c_void_p, c.c_void_p, c.c_uint32, Point, c.c_uint32)
        bind(self.cg, 'CGEventPostToPid', None, c.c_int, c.c_void_p)
        bind(self.cg, 'CGEventPost', None, c.c_uint32, c.c_void_p)
        bind(self.cg, 'CGEventSetIntegerValueField', None, c.c_void_p, c.c_int, c.c_int64)
        bind(self.cg, 'CGEventCreate', c.c_void_p, c.c_void_p)
        bind(self.cg, 'CGEventGetLocation', Point, c.c_void_p)
        self.held = set()

    def windows(self):
        array = self.cg.CGWindowListCopyWindowInfo(1, 0)  # On-screen windows, front to back.
        data = None
        try:
            data = self.cf.CFPropertyListCreateData(None, array, 100, 0, None)
            if not data:
                raise RuntimeError('could not read native window list')
            raw = c.string_at(self.cf.CFDataGetBytePtr(data), self.cf.CFDataGetLength(data))
            return [w for w in plistlib.loads(raw) if w.get('kCGWindowLayer') == 0]
        finally:
            if data:
                self.cf.CFRelease(data)
            if array:
                self.cf.CFRelease(array)

    def activate(self):
        app = self.ax.AXUIElementCreateApplication(self.pid)
        key = self.cf.CFStringCreateWithCString(None, b'AXFrontmost', 0x08000100)
        try:
            yes = c.c_void_p.in_dll(self.cf, 'kCFBooleanTrue')
            if self.ax.AXUIElementSetAttributeValue(app, key, yes) != 0:
                raise RuntimeError('could not activate the test game')
        finally:
            self.cf.CFRelease(key)
            self.cf.CFRelease(app)
        end = time.monotonic() + 5
        while time.monotonic() < end:
            windows = self.windows()
            if windows and windows[0].get('kCGWindowOwnerPID') == self.pid:
                return
            time.sleep(0.1)
        raise RuntimeError('the test game never became the frontmost window')

    def raise_window(self, title):
        # A background split window may be mostly covered by Window 1. Raise it
        # before clicking its content, like selecting it from the OS window menu.
        app = self.ax.AXUIElementCreateApplication(self.pid)
        keys = [self.cf.CFStringCreateWithCString(None, name, 0x08000100)
                for name in (b'AXWindows', b'AXTitle', b'AXRaise')]
        windows = c.c_void_p()
        try:
            if self.ax.AXUIElementCopyAttributeValue(app, keys[0], c.byref(windows)) != 0:
                raise RuntimeError('could not read the test games native windows')
            for i in range(self.cf.CFArrayGetCount(windows)):
                window = self.cf.CFArrayGetValueAtIndex(windows, i)
                name = c.c_void_p()
                if self.ax.AXUIElementCopyAttributeValue(window, keys[1], c.byref(name)) != 0:
                    continue
                try:
                    text = c.create_string_buffer(2048)
                    self.cf.CFStringGetCString(name, text, len(text), 0x08000100)
                    if title not in text.value.decode('utf-8'):
                        continue
                    if self.ax.AXUIElementPerformAction(window, keys[2]) != 0:
                        raise RuntimeError(f'could not raise {title}')
                    time.sleep(0.15)
                    return
                finally:
                    self.cf.CFRelease(name)
            raise RuntimeError(f'no accessible native window matching {title!r}')
        finally:
            if windows:
                self.cf.CFRelease(windows)
            for key in keys:
                self.cf.CFRelease(key)
            self.cf.CFRelease(app)

    def window(self, title=None):
        windows = [w for w in self.windows() if w.get('kCGWindowOwnerPID') == self.pid
                   and (title is None or title in w.get('kCGWindowName', ''))]
        if not windows:
            raise RuntimeError(f'no native test window matching {title!r}')
        return windows[0]

    def point(self, physical_size, x, y, title=None):
        """A fractional point in the content area, excluding the native title bar."""
        b = self.window(title)['kCGWindowBounds']
        scale = physical_size[0] / b['Width']
        height = physical_size[1] / scale
        return b['X'] + b['Width'] * x, b['Y'] + b['Height'] - height + height * y

    def key(self, key, down):
        event = self.cg.CGEventCreateKeyboardEvent(None, self.KEYS[key], down)
        try:
            # Target the test process; a disappearing test window must not type in another app.
            self.cg.CGEventPostToPid(self.pid, event)
            if down:
                self.held.add(key)
            else:
                self.held.discard(key)
        finally:
            self.cf.CFRelease(event)

    def tap(self, key):
        self.key(key, True)
        time.sleep(0.05)
        self.key(key, False)
        time.sleep(0.15)

    def click(self, at, title=None):
        self.activate()
        if title is not None:
            self.raise_window(title)
        self.move(at)
        for kind in (1, 2):  # Left down, left up.
            event = self.cg.CGEventCreateMouseEvent(None, kind, Point(*at), 0)
            self.cg.CGEventPost(0, event)
            self.cf.CFRelease(event)
            time.sleep(0.05)
        time.sleep(0.15)

    def move(self, at, delta=None):
        event = self.cg.CGEventCreateMouseEvent(None, 5, Point(*at), 0)
        try:
            if delta is not None:
                self.cg.CGEventSetIntegerValueField(event, 4, delta[0])
                self.cg.CGEventSetIntegerValueField(event, 5, delta[1])
            self.cg.CGEventPost(0, event)
        finally:
            self.cf.CFRelease(event)
        time.sleep(0.05)

    def relative(self, dx, dy):
        event = self.cg.CGEventCreate(None)
        at = self.cg.CGEventGetLocation(event)
        self.cf.CFRelease(event)
        self.move((at.x + dx, at.y + dy), (dx, dy))

    def release(self):
        for key in list(self.held):
            self.key(key, False)
