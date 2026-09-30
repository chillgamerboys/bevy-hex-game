// Only the explicitly requested game window is eligible. No display fallback,
// audio, microphone, image readback, or unbounded application frame queue.
import Foundation
import AppKit
import ScreenCaptureKit
import AVFoundation
import CoreGraphics

struct Request: Decodable {
    let command: String
    var pid: Int32?
    var title: String?
    var path: String?
}

func emit(_ event: String, _ fields: [String: Any] = [:]) {
    var object = fields
    object["event"] = event
    if let data = try? JSONSerialization.data(withJSONObject: object, options: [.sortedKeys]) {
        FileHandle.standardOutput.write(data)
        FileHandle.standardOutput.write(Data([10]))
    }
}

func outputSize(width: Double, height: Double) -> (Int, Int)? {
    guard width.isFinite, height.isFinite, width > 0, height > 0 else { return nil }
    let scale = min(1, min(1920 / width, 1080 / height))
    return (max(2, Int(width * scale) / 2 * 2), max(2, Int(height * scale) / 2 * 2))
}

@available(macOS 15.0, *)
@MainActor
final class Recorder: NSObject, SCRecordingOutputDelegate, SCStreamDelegate {
    var stream: SCStream?
    var output: SCRecordingOutput?
    var starting = false
    var stopping = false
    var stopPending = false
    var exitAfterStop = false
    var windowID: CGWindowID = 0
    var targetPID: Int32 = 0
    var finalURL: URL?
    var partialURL: URL?
    var startedAt: Date?
    let background = CGColor(gray: 0, alpha: 1)
    var monitor: Timer?
    var deadline: Timer?

    func receive(_ line: String) {
        guard line.utf8.count <= 16384, let data = line.data(using: .utf8),
              let request = try? JSONDecoder().decode(Request.self, from: data) else {
            fail("Invalid recorder control message.")
            return
        }
        switch request.command {
        case "start":
            guard !starting, stream == nil else { fail("Recording is already active."); return }
            starting = true
            deadline = Timer.scheduledTimer(withTimeInterval: 90, repeats: false) { [weak self] _ in
                Task { @MainActor in self?.fail("Recording did not start within 90 seconds.") }
            }
            Task { await begin(request) }
        case "stop": stop()
        case "quit": exitAfterStop = true; stop()
        default: fail("Unknown recorder command.")
        }
    }

    func begin(_ request: Request) async {
        do {
            guard let pid = request.pid, pid > 0, let title = request.title, !title.isEmpty,
                  let path = request.path, URL(fileURLWithPath: path).pathExtension == "mp4" else {
                throw RecorderError.message("Recording needs an exact process, window title and MP4 path.")
            }
            // This is reached only after the player's Record action. Refusal never
            // widens the filter or starts a screen/desktop capture.
            guard CGPreflightScreenCaptureAccess() || CGRequestScreenCaptureAccess() else {
                throw RecorderError.message("Allow Hex Game Recorder in System Settings → Privacy & Security → Screen & System Audio Recording, then try again. macOS may require restarting the game.")
            }
            let content = try await SCShareableContent.excludingDesktopWindows(true, onScreenWindowsOnly: true)
            if stopPending {
                emit("cancelled", ["message": "Recording canceled before capture began."])
                exit(0)
            }
            let matches = content.windows.filter {
                $0.owningApplication?.processID == pid && $0.title == title && $0.windowLayer == 0
            }
            guard matches.count == 1, let window = matches.first else {
                throw RecorderError.message("The exact battle window is missing or ambiguous. Recording was not started.")
            }
            let filter = SCContentFilter(desktopIndependentWindow: window)
            let scale = Double(filter.pointPixelScale)
            guard let (width, height) = outputSize(width: filter.contentRect.width * scale,
                                                   height: filter.contentRect.height * scale) else {
                throw RecorderError.message("The battle window has no valid visible size.")
            }
            let final = URL(fileURLWithPath: path)
            let partial = final.deletingPathExtension().appendingPathExtension("partial.mp4")
            guard !FileManager.default.fileExists(atPath: final.path),
                  !FileManager.default.fileExists(atPath: partial.path) else {
                throw RecorderError.message("Recording output already exists; refusing to overwrite it.")
            }
            let configuration = SCStreamConfiguration()
            configuration.width = width
            configuration.height = height
            configuration.minimumFrameInterval = CMTime(value: 1, timescale: 30)
            configuration.queueDepth = 3
            configuration.captureDynamicRange = .SDR
            configuration.scalesToFit = true
            configuration.preservesAspectRatio = true
            configuration.backgroundColor = background
            configuration.ignoreShadowsSingleWindow = true
            configuration.includeChildWindows = false
            configuration.showsCursor = true
            configuration.capturesAudio = false
            configuration.captureMicrophone = false
            let recording = SCRecordingOutputConfiguration()
            guard recording.availableVideoCodecTypes.contains(.h264),
                  recording.availableOutputFileTypes.contains(.mp4) else {
                throw RecorderError.message("This Mac does not offer H.264 MP4 recording.")
            }
            recording.outputURL = partial
            recording.outputFileType = .mp4
            recording.videoCodecType = .h264
            let stream = SCStream(filter: filter, configuration: configuration, delegate: self)
            let output = SCRecordingOutput(configuration: recording, delegate: self)
            try stream.addRecordingOutput(output)
            self.stream = stream
            self.output = output
            self.windowID = window.windowID
            self.targetPID = pid
            self.finalURL = final
            self.partialURL = partial
            emit("configured", ["window_id": windowID, "pid": pid, "width": width,
                                "height": height, "fps": 30, "audio": false, "codec": "h264", "path": final.path])
            try await stream.startCapture()
            // Only recordingOutputDidStartRecording acknowledges actual recording.
            if stopPending { stop() }
        } catch { fail(error.localizedDescription) }
    }

    func stop() {
        stopPending = true
        if starting { return }
        guard let stream else { if exitAfterStop { exit(0) }; return }
        guard !stopping else { return }
        stopping = true
        monitor?.invalidate()
        deadline?.invalidate()
        deadline = Timer.scheduledTimer(withTimeInterval: 30, repeats: false) { [weak self] _ in
            Task { @MainActor in self?.fail("Finalizing recording timed out; the partial file was retained.") }
        }
        Task {
            do { try await stream.stopCapture() }
            catch { fail(error.localizedDescription) }
        }
    }

    nonisolated func recordingOutputDidStartRecording(_ recordingOutput: SCRecordingOutput) {
        Task { @MainActor [self] in
            self.starting = false
            self.deadline?.invalidate()
            self.startedAt = Date()
            emit("started", ["window_id": self.windowID])
            self.monitor = Timer.scheduledTimer(withTimeInterval: 1, repeats: true) { [weak self] _ in
                Task { @MainActor in self?.checkLiveness() }
            }
            if self.stopPending { self.stop() }
        }
    }

    func checkLiveness() {
        // No repeated SCShareableContent enumeration, pixel readback or frame queue.
        // The stream delegate handles source-window destruction; process death,
        // and disk pressure have independent safeguards.
        guard NSRunningApplication(processIdentifier: targetPID) != nil else { stop(); return }
        if let finalURL,
           let capacity = try? finalURL.deletingLastPathComponent().resourceValues(forKeys: [.volumeAvailableCapacityForImportantUsageKey]).volumeAvailableCapacityForImportantUsage,
           capacity < 256 * 1024 * 1024 {
            emit("notice", ["message": "Low disk space; finalizing recording."])
            stop()
        }
    }

    nonisolated func recordingOutputDidFinishRecording(_ recordingOutput: SCRecordingOutput) {
        let duration = recordingOutput.recordedDuration.seconds
        Task { @MainActor in
            self.monitor?.invalidate()
            self.deadline?.invalidate()
            do {
                guard let source = self.partialURL, let destination = self.finalURL else {
                    throw RecorderError.message("Recording finished without an output path.")
                }
                let attributes = try FileManager.default.attributesOfItem(atPath: source.path)
                let size = (attributes[.size] as? NSNumber)?.int64Value ?? 0
                guard size > 0, duration.isFinite, duration > 0 else {
                    throw RecorderError.message("The recorder produced an empty clip.")
                }
                try FileManager.default.moveItem(at: source, to: destination)
                emit("finished", ["path": destination.path, "duration_seconds": duration, "bytes": size])
                exit(0)
            } catch { self.fail(error.localizedDescription) }
        }
    }

    nonisolated func recordingOutput(_ recordingOutput: SCRecordingOutput, didFailWithError error: Error) {
        let message = error.localizedDescription
        Task { @MainActor in self.fail(message) }
    }

    nonisolated func stream(_ stream: SCStream, didStopWithError error: Error) {
        let message = error.localizedDescription
        Task { @MainActor in
            // Never claim a valid movie from a stream-error callback. Only the
            // recording delegate can confirm successful finalization.
            self.fail(message)
        }
    }

    func fail(_ message: String) {
        monitor?.invalidate()
        deadline?.invalidate()
        emit("failed", ["message": message, "partial_path": partialURL?.path ?? ""])
        exit(1)
    }
}

enum RecorderError: LocalizedError {
    case message(String)
    var errorDescription: String? { if case let .message(text) = self { return text }; return nil }
}

@main
struct Main {
    @MainActor static func main() {
        if CommandLine.arguments.contains("--self-test") {
            let tests = [(outputSize(width: 3840, height: 2160) ?? (0, 0)) == (1920, 1080),
                         (outputSize(width: 1600, height: 900) ?? (0, 0)) == (1600, 900),
                         (outputSize(width: 900, height: 1600) ?? (0, 0)) == (606, 1080),
                         outputSize(width: .nan, height: 900) == nil,
                         outputSize(width: 0, height: 900) == nil]
            guard tests.allSatisfy({ $0 }) else { emit("failed", ["message": "geometry oracle"]); exit(1) }
            let request = try? JSONDecoder().decode(Request.self, from: Data("{\"command\":\"stop\"}".utf8))
            guard request?.command == "stop" else { exit(1) }
            emit("self_test_pass", ["tests": tests.count + 1, "capture_started": false])
            return
        }
        guard #available(macOS 15.0, *) else {
            emit("failed", ["message": "Recording requires macOS 15 or newer.", "code": "unsupported"])
            exit(1)
        }
        let recorder = Recorder()
        _ = NSApplication.shared
        NSApp.setActivationPolicy(.accessory)
        emit("ready")
        DispatchQueue.global(qos: .userInitiated).async {
            while let line = readLine() {
                Task { @MainActor in recorder.receive(line) }
            }
            Task { @MainActor in recorder.exitAfterStop = true; recorder.stop() }
        }
        RunLoop.main.run()
    }
}
