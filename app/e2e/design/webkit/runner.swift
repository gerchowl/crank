// The WKWebView half of the design audit (#630, webkit.ts): the system WebKit the app ships in (Tauri's WKWebView on
// macOS) lays out some things unlike the harness's Chromium. Usage: `runner <width> <height> <driver.js>`, then one
// JSON job per stdin line ({"url", "init", "job", "audit", "opts"}); prints one line per job, the driver's JSON or `{"error"}`.
// Needs the logged-in GUI session (a WebView does), like `just quit-test`.
import Cocoa
import WebKit

let args = CommandLine.arguments
guard args.count == 4, let w = Double(args[1]), let h = Double(args[2]),
  let driver = try? String(contentsOfFile: args[3], encoding: .utf8)
else {
  FileHandle.standardError.write("usage: runner <width> <height> <driver.js>\n".data(using: .utf8)!)
  exit(2)
}

final class Run: NSObject, WKNavigationDelegate {
  let view: WKWebView
  let driver: String
  var jobs: [[String: Any]]
  var current: [String: Any] = [:]

  init(view: WKWebView, driver: String, jobs: [[String: Any]]) {
    self.view = view
    self.driver = driver
    self.jobs = jobs
    super.init()
    view.navigationDelegate = self
  }

  func next() {
    guard !jobs.isEmpty, let url = URL(string: jobs[0]["url"] as? String ?? "") else { exit(0) }
    current = jobs.removeFirst()
    // Before the page's own scripts, as Playwright's `addInitScript` (the frozen clock).
    let scripts = view.configuration.userContentController
    scripts.removeAllUserScripts()
    if let source = current["init"] as? String {
      scripts.addUserScript(WKUserScript(source: source, injectionTime: .atDocumentStart, forMainFrameOnly: true))
    }
    view.load(URLRequest(url: url))
  }

  func emit(_ line: String) {
    print(line)
    fflush(stdout)
    next()
  }

  func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
    let input = ["job": current["job"] ?? [:], "audit": current["audit"] ?? "", "opts": current["opts"] ?? [:]]
    webView.callAsyncJavaScript(driver, arguments: input, in: nil, in: .page) { result in
      switch result {
      case .success(let value): self.emit(value as? String ?? #"{"error":"the driver returned no JSON"}"#)
      case .failure(let error): self.emit(Self.error("\(error)"))
      }
    }
  }

  func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
    emit(Self.error("\(error)"))
  }

  func webView(
    _ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error
  ) {
    emit(Self.error("\(error)"))
  }

  static func error(_ message: String) -> String {
    let data = try? JSONSerialization.data(withJSONObject: ["error": message])
    return data.flatMap { String(data: $0, encoding: .utf8) } ?? #"{"error":"unprintable"}"#
  }
}

var jobs: [[String: Any]] = []
while let line = readLine() {
  if let data = line.data(using: .utf8),
    let job = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
  {
    jobs.append(job)
  }
}
let app = NSApplication.shared
app.setActivationPolicy(.prohibited)
// Off screen, never shown to the rider: the window gives the view its size. WebKit stops the frames of a view it
// sees occluded (the fixture's `settled()` waits on them), so occlusion detection is off (WKWebView's private
// `_windowOcclusionDetectionEnabled`, the switch WebKit's own test runners use).
let window = NSWindow(
  contentRect: NSRect(x: -10000, y: 0, width: w, height: h), styleMask: [.borderless], backing: .buffered,
  defer: false)
let view = WKWebView(frame: NSRect(x: 0, y: 0, width: w, height: h))
let occlusion = NSSelectorFromString("_setWindowOcclusionDetectionEnabled:")
guard view.responds(to: occlusion) else {
  FileHandle.standardError.write("runner: this WebKit has no _setWindowOcclusionDetectionEnabled:\n".data(using: .utf8)!)
  exit(2)
}
typealias SetBool = @convention(c) (AnyObject, Selector, Bool) -> Void
unsafeBitCast(view.method(for: occlusion), to: SetBool.self)(view, occlusion, false)
window.contentView = view
window.orderFront(nil)
let run = Run(view: view, driver: driver, jobs: jobs)
run.next()
app.run()
