// opensheet-translate: the bridge between OpenSheet and the translation
// built into macOS (Apple's Translation framework has no C or Objective-C
// interface, so the application runs this program for each request).
//
//   opensheet-translate languages
//       -> "ok", then one language code per line
//   opensheet-translate translate <from|auto> <to>     (text on standard input)
//       -> "ok <source language>", then the translation
//
// Anything that goes wrong is one line: "error <code> <detail>", with the
// codes unsupported-system, not-installed, unsupported-pair,
// unknown-language and failed. Nothing leaves the machine: the framework
// translates with the languages downloaded in System Settings.
//
// Built by scripts/build_translate_helper.sh.

import Foundation
import NaturalLanguage

#if canImport(Translation)
import Translation
#endif

func answer(_ first: String, _ rest: String = "") -> Never {
    print(first)
    if !rest.isEmpty {
        print(rest)
    }
    exit(0)
}

func fail(_ code: String, _ detail: String = "") -> Never {
    answer("error \(code) \(detail.replacingOccurrences(of: "\n", with: " "))")
}

/// The language most of a text is written in.
func detect(_ text: String) -> String? {
    let recognizer = NLLanguageRecognizer()
    recognizer.processString(text)
    return recognizer.dominantLanguage?.rawValue
}

@main
struct Helper {
    static func main() async {
        let arguments = Array(CommandLine.arguments.dropFirst())
        #if canImport(Translation)
        guard #available(macOS 26.0, *) else {
            // Before macOS 26 the framework only translates inside a
            // SwiftUI view: there is nothing a helper can call.
            fail("unsupported-system")
        }
        switch arguments.first {
        case "languages":
            let languages = await LanguageAvailability().supportedLanguages
            let codes = Set(languages.map { $0.minimalIdentifier }).sorted()
            answer("ok", codes.joined(separator: "\n"))
        case "translate" where arguments.count == 3:
            let text = String(decoding: FileHandle.standardInput.readDataToEndOfFile(), as: UTF8.self)
            guard let from = arguments[1] == "auto" ? detect(text) : arguments[1] else {
                fail("unknown-language")
            }
            let source = Locale.Language(identifier: from)
            let target = Locale.Language(identifier: arguments[2])
            switch await LanguageAvailability().status(from: source, to: target) {
            case .installed:
                break
            case .supported:
                fail("not-installed", "\(from) \(arguments[2])")
            default:
                fail("unsupported-pair", "\(from) \(arguments[2])")
            }
            do {
                let session = TranslationSession(installedSource: source, target: target)
                let response = try await session.translate(text)
                answer("ok \(from)", response.targetText)
            } catch {
                fail("failed", error.localizedDescription)
            }
        default:
            fail("failed", "usage: opensheet-translate languages | translate <from|auto> <to>")
        }
        #else
        fail("unsupported-system")
        #endif
    }
}
