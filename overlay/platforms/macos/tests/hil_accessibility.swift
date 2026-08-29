// Copyright 2026 sunaemon
// SPDX-License-Identifier: MIT

import AppKit
import ApplicationServices
import Foundation

private struct Configuration {
    let overlayPID: pid_t
    let driver: String
    let keyboardID: UInt8
    let layer: UInt8
    let secondaryLayer: UInt8
    let expectedLabel: String

    static func parse() throws -> Configuration {
        var values: [String: String] = [:]
        var arguments = Array(CommandLine.arguments.dropFirst())
        while !arguments.isEmpty {
            let option = arguments.removeFirst()
            guard option.hasPrefix("--"), !arguments.isEmpty else {
                throw Failure("Expected --option value pairs")
            }
            values[option] = arguments.removeFirst()
        }

        guard
            let pidText = values["--overlay-pid"],
            let overlayPID = pid_t(pidText),
            let driver = values["--driver"],
            let keyboardText = values["--keyboard-id"],
            let keyboardID = UInt8(keyboardText),
            let layerText = values["--layer"],
            let layer = UInt8(layerText),
            let secondaryLayerText = values["--secondary-layer"],
            let secondaryLayer = UInt8(secondaryLayerText),
            let expectedLabel = values["--expected-label"]
        else {
            throw Failure(
                "Required: --overlay-pid PID --driver PATH --keyboard-id ID "
                    + "--layer LAYER --expected-label LABEL")
        }
        guard secondaryLayer > layer else {
            throw Failure("--secondary-layer must be numerically above --layer")
        }
        return Configuration(
            overlayPID: overlayPID,
            driver: driver,
            keyboardID: keyboardID,
            layer: layer,
            secondaryLayer: secondaryLayer,
            expectedLabel: expectedLabel)
    }
}

private struct Failure: Error, CustomStringConvertible {
    let description: String

    init(_ description: String) {
        self.description = description
    }
}

private final class ProbeView: NSView {
    private(set) var clickCount = 0

    override func mouseDown(with event: NSEvent) {
        clickCount += 1
    }
}

private final class ProbeWindow: NSWindow {
    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { true }
}

private final class Runner {
    private let configuration: Configuration
    private let probe = ProbeView()
    private let textField = NSTextField()
    private let window: ProbeWindow

    init(configuration: Configuration) throws {
        self.configuration = configuration
        guard let screen = NSScreen.main else {
            throw Failure("No macOS screen is available")
        }
        window = ProbeWindow(
            contentRect: screen.frame,
            styleMask: [.borderless],
            backing: .buffered,
            defer: false)
        window.level = .normal
        window.backgroundColor = .windowBackgroundColor
        window.contentView = probe

        textField.frame = NSRect(x: 40, y: screen.frame.height - 100, width: 500, height: 40)
        textField.stringValue = ""
        probe.addSubview(textField)
    }

    func run() throws {
        guard AXIsProcessTrusted() else {
            throw Failure(
                "Accessibility permission is required for the stable HIL UI binary")
        }
        window.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        window.makeFirstResponder(textField)
        pumpRunLoop(for: 0.5)

        try sendLayer(state: "press")
        let overlay = try waitForOverlay(visible: true)
        try assertFocusAndTyping()
        try assertTopmost(overlayWindowNumber: overlay.number)
        try assertClickThrough(overlayBounds: overlay.bounds)
        try assertLabel(configuration.expectedLabel)
        try assertCenteredOnAvailableDisplays()
        try assertRepeatedHolds()
        try assertLayerPrecedence()
        print("macOS Accessibility HIL checks passed")
    }

    private func sendLayer(state: String, layer: UInt8? = nil) throws {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: configuration.driver)
        process.arguments = [
            "layer",
            "--keyboard-id", String(configuration.keyboardID),
            "--layer", String(layer ?? configuration.layer),
            "--state", state,
        ]
        try process.run()
        process.waitUntilExit()
        guard process.terminationStatus == 0 else {
            throw Failure("HIL driver failed while sending layer (state)")
        }
        pumpRunLoop(for: 0.25)
    }

    private func assertFocusAndTyping() throws {
        guard NSWorkspace.shared.frontmostApplication?.processIdentifier == getpid() else {
            throw Failure("The overlay took application focus")
        }
        guard window.isKeyWindow, window.firstResponder === textField.currentEditor() else {
            throw Failure("The overlay moved focus away from the text field")
        }

        let marker = "focus-through-overlay"
        postText(marker)
        pumpRunLoop(for: 0.25)
        guard textField.stringValue == marker else {
            throw Failure("Injected typing did not remain in the focused text field")
        }
    }

    private func assertTopmost(overlayWindowNumber: Int) throws {
        let windows = windowList()
        guard
            let overlayIndex = windows.firstIndex(where: { $0.number == overlayWindowNumber }),
            let probeIndex = windows.firstIndex(where: { $0.pid == getpid() })
        else {
            throw Failure("Could not compare overlay and probe window ordering")
        }
        guard overlayIndex < probeIndex else {
            throw Failure("The overlay is not above the active application")
        }
    }

    private func assertClickThrough(overlayBounds: CGRect) throws {
        let before = probe.clickCount
        let point = CGPoint(x: overlayBounds.midX, y: overlayBounds.midY)
        postMouseClick(at: point)
        pumpRunLoop(for: 0.25)
        guard probe.clickCount == before + 1 else {
            throw Failure("A click inside the overlay did not reach the underlying application")
        }
        guard NSWorkspace.shared.frontmostApplication?.processIdentifier == getpid() else {
            throw Failure("Clicking through the overlay changed application focus")
        }
    }

    private func assertLabel(_ expected: String) throws {
        let application = AXUIElementCreateApplication(configuration.overlayPID)
        let labels = accessibilityStrings(in: application, depth: 0)
        guard labels.contains(expected) else {
            throw Failure(
                "Overlay Accessibility labels do not contain \(expected); "
                    + "observed: \(labels.sorted())")
        }
    }

    private func assertRepeatedHolds() throws {
        try sendLayer(state: "release")
        _ = try waitForOverlay(visible: false)
        for _ in 0..<10 {
            try sendLayer(state: "press")
            _ = try waitForOverlay(visible: true)
            try assertLabel("L\(configuration.layer)")
            try sendLayer(state: "release")
            _ = try waitForOverlay(visible: false)
        }
    }

    private func assertLayerPrecedence() throws {
        try sendLayer(state: "press")
        _ = try waitForOverlay(visible: true)
        try assertLabel("L\(configuration.layer)")
        try sendLayer(state: "press", layer: configuration.secondaryLayer)
        _ = try waitForOverlay(visible: true)
        try assertLabel("L\(configuration.secondaryLayer)")
        try sendLayer(state: "release", layer: configuration.secondaryLayer)
        _ = try waitForOverlay(visible: true)
        try assertLabel("L\(configuration.layer)")
        try sendLayer(state: "release")
        _ = try waitForOverlay(visible: false)
    }

    private func assertCenteredOnAvailableDisplays() throws {
        var displayCount: UInt32 = 0
        CGGetActiveDisplayList(0, nil, &displayCount)
        var displays = [CGDirectDisplayID](repeating: 0, count: Int(displayCount))
        CGGetActiveDisplayList(displayCount, &displays, &displayCount)
        guard !displays.isEmpty else {
            throw Failure("No active displays were reported")
        }

        for display in displays {
            let displayBounds = CGDisplayBounds(display)
            let point = CGPoint(x: displayBounds.midX, y: displayBounds.midY)
            CGWarpMouseCursorPosition(point)
            try sendLayer(state: "release")
            try sendLayer(state: "press")
            let overlay = try waitForOverlay(visible: true)
            let tolerance = 2.0
            guard
                abs(overlay.bounds.midX - displayBounds.midX) <= tolerance,
                abs(overlay.bounds.midY - displayBounds.midY) <= tolerance
            else {
                throw Failure(
                    "Overlay is not centered on display \(display): "
                        + "overlay=\(overlay.bounds) display=\(displayBounds)")
            }
        }
    }

    private func waitForOverlay(visible: Bool) throws -> WindowRecord {
        let deadline = Date().addingTimeInterval(5)
        repeat {
            if let record = windowList().first(where: { $0.pid == configuration.overlayPID }) {
                let isVisible = record.bounds.width > 1 && record.bounds.height > 1
                if isVisible == visible {
                    return record
                }
            }
            pumpRunLoop(for: 0.05)
        } while Date() < deadline
        throw Failure("Timed out waiting for overlay visible=\(visible)")
    }
}

private struct WindowRecord {
    let pid: pid_t
    let number: Int
    let bounds: CGRect
}

private func windowList() -> [WindowRecord] {
    guard let entries = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID)
        as? [[String: Any]]
    else {
        return []
    }
    return entries.compactMap { entry in
        guard
            let pid = (entry[kCGWindowOwnerPID as String] as? NSNumber)?.int32Value,
            let number = (entry[kCGWindowNumber as String] as? NSNumber)?.intValue,
            let boundsDictionary = entry[kCGWindowBounds as String] as? NSDictionary,
            let bounds = CGRect(dictionaryRepresentation: boundsDictionary as CFDictionary)
        else {
            return nil
        }
        return WindowRecord(pid: pid, number: number, bounds: bounds)
    }
}

private func accessibilityStrings(in element: AXUIElement, depth: Int) -> [String] {
    guard depth < 20 else {
        return []
    }
    var strings: [String] = []
    for attribute in [kAXValueAttribute, kAXTitleAttribute, kAXDescriptionAttribute] {
        var value: CFTypeRef?
        if AXUIElementCopyAttributeValue(element, attribute as CFString, &value) == .success,
            let string = value as? String,
            !string.isEmpty
        {
            strings.append(string)
        }
    }
    for attribute in [kAXWindowsAttribute, kAXChildrenAttribute] {
        var value: CFTypeRef?
        guard
            AXUIElementCopyAttributeValue(element, attribute as CFString, &value) == .success,
            let children = value as? [AXUIElement]
        else {
            continue
        }
        strings.append(contentsOf: children.flatMap { accessibilityStrings(in: $0, depth: depth + 1) })
    }
    return strings
}

private func postText(_ text: String) {
    let characters = Array(text.utf16)
    characters.withUnsafeBufferPointer { buffer in
        for keyDown in [true, false] {
            guard let event = CGEvent(keyboardEventSource: nil, virtualKey: 0, keyDown: keyDown) else {
                continue
            }
            event.keyboardSetUnicodeString(
                stringLength: characters.count,
                unicodeString: buffer.baseAddress)
            event.post(tap: .cghidEventTap)
        }
    }
}

private func postMouseClick(at point: CGPoint) {
    for type in [CGEventType.leftMouseDown, CGEventType.leftMouseUp] {
        CGEvent(
            mouseEventSource: nil,
            mouseType: type,
            mouseCursorPosition: point,
            mouseButton: .left)?.post(tap: .cghidEventTap)
    }
}

private func pumpRunLoop(for seconds: TimeInterval) {
    RunLoop.current.run(until: Date().addingTimeInterval(seconds))
}

private func fail(_ error: Error) -> Never {
    FileHandle.standardError.write(Data("ERROR: \(error)\n".utf8))
    exit(1)
}

if CommandLine.arguments == [CommandLine.arguments[0], "--check-accessibility"] {
    if AXIsProcessTrusted() {
        print("Accessibility permission is available")
        exit(0)
    }
    fail(Failure("Accessibility permission is not granted to this HIL UI binary"))
}

private let configuration: Configuration
do {
    configuration = try Configuration.parse()
} catch {
    fail(error)
}

let application = NSApplication.shared
application.setActivationPolicy(.regular)
private let runner: Runner
do {
    runner = try Runner(configuration: configuration)
} catch {
    fail(error)
}
DispatchQueue.main.async {
    do {
        try runner.run()
        NSApp.terminate(nil)
    } catch {
        fail(error)
    }
}
application.run()
