// Presses a key combination the way a keyboard would, for the macOS smoke
// test: modifiers send flags-changed events, other keys key-down/up.
//
//   post-keys ctrl+shift+40 [hold_ms]
//   post-keys capslock-on | capslock-off
//
// Parts are modifier names (ctrl, shift, alt, cmd, fn) or virtual key codes
// (40 = K). They go down in order, stay held for hold_ms, then come up in
// reverse order. Caps Lock is a single flags-changed event per toggle, as
// macOS reports it.
import CoreGraphics
import Foundation

let modifiers: [String: (CGKeyCode, CGEventFlags)] = [
    "ctrl": (0x3B, .maskControl),
    "shift": (0x38, .maskShift),
    "alt": (0x3A, .maskAlternate),
    "cmd": (0x37, .maskCommand),
    "fn": (0x3F, .maskSecondaryFn),
]

let args = CommandLine.arguments
guard args.count >= 2 else {
    FileHandle.standardError.write("usage: post-keys ctrl+shift+40 [hold_ms]\n".data(using: .utf8)!)
    exit(2)
}
let parts = args[1].split(separator: "+").map(String.init)
let holdMs = args.count > 2 ? UInt32(args[2]) ?? 100 : 100
let source = CGEventSource(stateID: .hidSystemState)
var flags: CGEventFlags = []

func post(_ code: CGKeyCode, down: Bool, flagsChanged: Bool) {
    guard let event = CGEvent(keyboardEventSource: source, virtualKey: code, keyDown: down) else {
        FileHandle.standardError.write("could not create a key event\n".data(using: .utf8)!)
        exit(1)
    }
    if flagsChanged {
        event.type = .flagsChanged
    }
    event.flags = flags
    event.post(tap: .cghidEventTap)
    usleep(40_000)
}

if args[1] == "capslock-on" || args[1] == "capslock-off" {
    flags = args[1] == "capslock-on" ? [.maskAlphaShift] : []
    post(0x39, down: true, flagsChanged: true)
    exit(0)
}

for part in parts {
    if let m = modifiers[part] {
        flags.insert(m.1)
        post(m.0, down: true, flagsChanged: true)
    } else if let code = CGKeyCode(part) {
        post(code, down: true, flagsChanged: false)
    }
}
usleep(holdMs * 1000)
for part in parts.reversed() {
    if let m = modifiers[part] {
        flags.remove(m.1)
        post(m.0, down: false, flagsChanged: true)
    } else if let code = CGKeyCode(part) {
        post(code, down: false, flagsChanged: false)
    }
}
