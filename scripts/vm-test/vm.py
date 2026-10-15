#!/usr/bin/env python3
"""Runs and drives headless test VMs (QEMU + KVM) for JellyBridge.

Nothing appears on the host's screen: the display is egl-headless (virgl GL
on a host render node) or plain virtio-vga (guest software GL), and the VM is
driven through QMP.

    vm.py start NAME ISO [--gl virgl|none] [--mem 8G] [--cpus 4]
    vm.py shot  NAME OUT.png           screenshot of the guest screen
    vm.py click NAME X Y [--double]    primary click at guest pixels
    vm.py move  NAME X Y
    vm.py type  NAME TEXT              type text (US layout); "\\n" types Enter
    vm.py key   NAME COMBO             e.g. ctrl-alt-t, meta_l, ret, esc
    vm.py stop  NAME
    vm.py list

State lives in ~/.local/share/jellybridge-vmtest/<NAME>/.
"""
import argparse, json, os, shutil, socket, struct, subprocess, sys, time

STATE = os.path.expanduser(os.environ.get("JB_VMTEST_DIR", "~/.local/share/jellybridge-vmtest"))
OVMF_CODE = "/usr/share/edk2/x64/OVMF_CODE.4m.fd"
OVMF_VARS = "/usr/share/edk2/x64/OVMF_VARS.4m.fd"
# The AMD iGPU's render node by default: Mesa's virgl host path is the most
# reliable, and it keeps the NVIDIA card free for the user.
RENDER_NODE = os.environ.get("JB_VMTEST_RENDER_NODE", "/dev/dri/renderD129")


def vm_dir(name):
    return os.path.join(STATE, name)


class Qmp:
    def __init__(self, name):
        self.sock = socket.socket(socket.AF_UNIX)
        self.sock.settimeout(30)
        self.sock.connect(os.path.join(vm_dir(name), "qmp.sock"))
        self.buf = b""
        self._read()  # greeting
        self.cmd("qmp_capabilities")

    def _read(self):
        while b"\n" not in self.buf:
            chunk = self.sock.recv(65536)
            if not chunk:
                raise SystemExit("QMP connection closed")
            self.buf += chunk
        line, self.buf = self.buf.split(b"\n", 1)
        return json.loads(line)

    def cmd(self, execute, **arguments):
        msg = {"execute": execute}
        if arguments:
            msg["arguments"] = arguments
        self.sock.sendall(json.dumps(msg).encode() + b"\n")
        while True:
            reply = self._read()
            if "return" in reply:
                return reply["return"]
            if "error" in reply:
                raise SystemExit(f"QMP {execute}: {reply['error']['desc']}")


def start(args):
    d = vm_dir(args.name)
    os.makedirs(d, exist_ok=True)
    vars_fd = os.path.join(d, "vars.fd")
    if not os.path.exists(vars_fd):
        shutil.copy(OVMF_VARS, vars_fd)
    if args.gl == "virgl":
        display = ["-device", "virtio-vga-gl", "-display", f"egl-headless,rendernode={RENDER_NODE}",
                   "-vnc", f"unix:{d}/vnc.sock"]
    else:
        display = ["-device", "virtio-vga", "-display", "none"]
    cmd = [
        "qemu-system-x86_64", "-name", f"jb-{args.name}", "-enable-kvm", "-machine", "q35",
        "-cpu", "host", "-smp", str(args.cpus), "-m", args.mem,
        "-drive", f"if=pflash,format=raw,readonly=on,file={OVMF_CODE}",
        "-drive", f"if=pflash,format=raw,file={vars_fd}",
        "-drive", f"media=cdrom,readonly=on,file={os.path.abspath(args.iso)}",
        *display,
        "-device", "qemu-xhci", "-device", "usb-tablet",
        "-nic", "user,model=virtio-net-pci",
        "-qmp", f"unix:{d}/qmp.sock,server=on,wait=off",
        "-monitor", "none", "-serial", "none", "-parallel", "none",
        "-daemonize", "-pidfile", f"{d}/pid",
    ]
    if args.disk:
        disk = os.path.join(d, "disk.qcow2")
        if not os.path.exists(disk):
            subprocess.run(["qemu-img", "create", "-q", "-f", "qcow2", disk, args.disk], check=True)
        cmd += ["-drive", f"if=virtio,file={disk}"]
    subprocess.run(cmd, check=True)
    print(f"started jb-{args.name} ({args.gl})")


def screen_size(name):
    path = os.path.join(vm_dir(name), "size.json")
    if not os.path.exists(path):
        shot_to(name, os.path.join(vm_dir(name), "last.png"))
    return json.load(open(path))


def recv_exact(sock, n):
    data = bytearray()
    while len(data) < n:
        chunk = sock.recv(n - len(data))
        if not chunk:
            raise SystemExit("VNC connection closed")
        data += chunk
    return bytes(data)


def vnc_capture(path):
    """One full frame over QEMU's local VNC socket (RFB 3.8, raw encoding).

    QMP screendump has no surface once a virgl guest scans out through GL;
    the VNC server reads the GL framebuffer back, so it sees what a user would.
    """
    from PIL import Image
    s = socket.socket(socket.AF_UNIX)
    s.settimeout(30)
    s.connect(path)
    recv_exact(s, 12)
    s.sendall(b"RFB 003.008\n")
    count = recv_exact(s, 1)[0]
    if 1 not in recv_exact(s, count):
        raise SystemExit("VNC server wants authentication")
    s.sendall(b"\x01")
    if struct.unpack(">I", recv_exact(s, 4))[0] != 0:
        raise SystemExit("VNC security handshake failed")
    s.sendall(b"\x01")  # shared
    w, h = struct.unpack(">HH", recv_exact(s, 4))
    recv_exact(s, 16)
    recv_exact(s, struct.unpack(">I", recv_exact(s, 4))[0])
    # 32 bpp little-endian BGRX, then raw encoding only.
    s.sendall(struct.pack(">BxxxBBBBHHHBBBxxx", 0, 32, 24, 0, 1, 255, 255, 255, 16, 8, 0))
    s.sendall(struct.pack(">BxHi", 2, 1, 0))
    s.sendall(struct.pack(">BBHHHH", 3, 0, 0, 0, w, h))
    frame = bytearray(w * h * 4)
    covered = 0
    while covered < w * h:
        kind = recv_exact(s, 1)[0]
        if kind != 0:  # bell / cut text: skip
            if kind == 2:
                continue
            if kind == 3:
                recv_exact(s, 3)
                recv_exact(s, struct.unpack(">I", recv_exact(s, 4))[0])
                continue
            raise SystemExit(f"unexpected VNC message {kind}")
        recv_exact(s, 1)
        for _ in range(struct.unpack(">H", recv_exact(s, 2))[0]):
            x, y, rw, rh, enc = struct.unpack(">HHHHi", recv_exact(s, 12))
            if enc == -223:  # desktop size
                w, h, frame, covered = rw, rh, bytearray(rw * rh * 4), 0
                s.sendall(struct.pack(">BBHHHH", 3, 0, 0, 0, w, h))
                continue
            data = recv_exact(s, rw * rh * 4)
            for row in range(rh):
                start = ((y + row) * w + x) * 4
                frame[start:start + rw * 4] = data[row * rw * 4:(row + 1) * rw * 4]
            covered += rw * rh
    s.close()
    Image.frombuffer("RGBX", (w, h), bytes(frame), "raw", "BGRX").convert("RGB").save(path_out := path + ".png")
    return path_out, w, h


def shot_to(name, out):
    sock = os.path.join(vm_dir(name), "vnc.sock")
    if os.path.exists(sock):
        tmp, w, h = vnc_capture(sock)
        shutil.move(tmp, out)
    else:
        q = Qmp(name)
        tmp = os.path.join(vm_dir(name), "screen.png")
        q.cmd("screendump", filename=tmp, format="png")
        for _ in range(50):
            if os.path.exists(tmp) and os.path.getsize(tmp) > 0:
                break
            time.sleep(0.1)
        time.sleep(0.2)
        shutil.copy(tmp, out)
        with open(out, "rb") as f:
            head = f.read(24)
        w, h = struct.unpack(">II", head[16:24])
    json.dump({"w": w, "h": h}, open(os.path.join(vm_dir(name), "size.json"), "w"))
    return w, h


def shot(args):
    w, h = shot_to(args.name, args.out)
    print(f"{args.out} {w}x{h}")


def pointer(q, name, x, y):
    size = screen_size(name)
    q.cmd("input-send-event", events=[
        {"type": "abs", "data": {"axis": "x", "value": int(x * 32767 / max(size["w"] - 1, 1))}},
        {"type": "abs", "data": {"axis": "y", "value": int(y * 32767 / max(size["h"] - 1, 1))}},
    ])


def move(args):
    pointer(Qmp(args.name), args.name, args.x, args.y)


def click(args):
    q = Qmp(args.name)
    pointer(q, args.name, args.x, args.y)
    time.sleep(0.1)
    for _ in range(2 if args.double else 1):
        for down in (True, False):
            q.cmd("input-send-event", events=[{"type": "btn", "data": {"down": down, "button": args.button}}])
            time.sleep(0.05)


SHIFTED = {'!': '1', '@': '2', '#': '3', '$': '4', '%': '5', '^': '6', '&': '7', '*': '8', '(': '9', ')': '0',
           '_': 'minus', '+': 'equal', '{': 'bracket_left', '}': 'bracket_right', '|': 'backslash',
           ':': 'semicolon', '"': 'apostrophe', '<': 'comma', '>': 'dot', '?': 'slash', '~': 'grave_accent'}
PLAIN = {' ': 'spc', '-': 'minus', '=': 'equal', '[': 'bracket_left', ']': 'bracket_right', '\\': 'backslash',
         ';': 'semicolon', "'": 'apostrophe', ',': 'comma', '.': 'dot', '/': 'slash', '`': 'grave_accent',
         '\n': 'ret', '\t': 'tab'}


def keys_for(c):
    if c.isascii() and c.isalpha():
        return (["shift"] if c.isupper() else []) + [c.lower()]
    if c.isdigit():
        return [c]
    if c in PLAIN:
        return [PLAIN[c]]
    if c in SHIFTED:
        return ["shift", SHIFTED[c]]
    raise SystemExit(f"cannot type {c!r}")


def send(q, names):
    q.cmd("send-key", keys=[{"type": "qcode", "data": k} for k in names], **{"hold-time": 40})
    time.sleep(0.05)


def type_(args):
    q = Qmp(args.name)
    for c in args.text.replace("\\n", "\n"):
        send(q, keys_for(c))


def key(args):
    q = Qmp(args.name)
    alias = {"ctrl": "ctrl", "alt": "alt", "shift": "shift", "super": "meta_l", "enter": "ret"}
    send(q, [alias.get(k, k) for k in args.combo.split("-")])


def stop(args):
    d = vm_dir(args.name)
    try:
        Qmp(args.name).cmd("quit")
    except (OSError, SystemExit):
        pid = os.path.join(d, "pid")
        if os.path.exists(pid):
            subprocess.run(["kill", open(pid).read().strip()])
    print(f"stopped jb-{args.name}")


def list_(args):
    subprocess.run("pgrep -a -f 'qemu-system-x86_64 -name jb-' || echo 'no test VMs running'", shell=True)


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = p.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("start"); s.add_argument("name"); s.add_argument("iso")
    s.add_argument("--gl", choices=["virgl", "none"], default="virgl")
    s.add_argument("--mem", default="8G"); s.add_argument("--cpus", type=int, default=4)
    s.add_argument("--disk", help="also attach an empty disk of this size, e.g. 40G")
    s.set_defaults(fn=start)
    s = sub.add_parser("shot"); s.add_argument("name"); s.add_argument("out"); s.set_defaults(fn=shot)
    for n, fn in (("click", click), ("move", move)):
        s = sub.add_parser(n); s.add_argument("name"); s.add_argument("x", type=float); s.add_argument("y", type=float)
        if n == "click":
            s.add_argument("--double", action="store_true"); s.add_argument("--button", default="left")
        s.set_defaults(fn=fn)
    s = sub.add_parser("type"); s.add_argument("name"); s.add_argument("text"); s.set_defaults(fn=type_)
    s = sub.add_parser("key"); s.add_argument("name"); s.add_argument("combo"); s.set_defaults(fn=key)
    s = sub.add_parser("stop"); s.add_argument("name"); s.set_defaults(fn=stop)
    s = sub.add_parser("list"); s.set_defaults(fn=list_)
    a = p.parse_args()
    a.fn(a)


if __name__ == "__main__":
    main()
