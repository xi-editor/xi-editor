// Copyright 2017 Google Inc. All rights reserved.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

import Cocoa

/// A represents a given text style.
struct Style {
    var fgColor: NSColor;
    var bgColor: NSColor;
    var weight: UInt16;
    var underline: Bool;
    var italic: Bool;
}

typealias StyleIdentifier = Int

/// A basic type representing a range of text and and a style identifier.
struct StyleSpan {
    let range: NSRange
    let style: StyleIdentifier

    /// given a line of text and an array of style values, generate an array of StyleSpans.
    /// see https://github.com/google/xi-editor/blob/protocol_doc/doc/update.md
    static func styles(fromRaw raw: [Int], text: String) -> [StyleSpan] {
        var out: [StyleSpan] = [];
        var ix = 0;
        for i in stride(from: 0, to: raw.count, by: 3) {
            let start = ix + raw[i]
            let end = start + raw[i + 1]
            let style = raw[i + 2]
            let startIx = utf8_offset_to_utf16(text, start)
            let endIx = utf8_offset_to_utf16(text, end)
            if startIx < 0 || endIx < startIx {
                //FIXME: how should we be doing error handling?
                print("malformed style array for line:", text, raw)
            } else {
                out.append(StyleSpan.init(range: NSMakeRange(startIx, endIx - startIx), style: style))
            }
            ix = end
        }
        return out
    }
}

func utf8_offset_to_utf16(_ s: String, _ ix: Int) -> Int {
    return s.utf8.index(s.utf8.startIndex, offsetBy: ix).samePosition(in: s.utf16)!._offset
}

/// A store of text styles, indexable by id.
/// - Note: all public methods of this class are designed to be thread-safe.
class StyleMap {
    private let queue = DispatchQueue(label: "com.levien.xi.StyleMap")
    private var map: [Style?] = []

    func defStyle(json: [String: AnyObject]) {
        queue.sync {
            defStyleLocked(json: json)
        }
    }

    private func defStyleLocked(json: [String: AnyObject]) {
        //print("defStyle: \(json)")
        guard let id = json["id"] as? Int else { return }
        var fgColor: UInt32 = 0xFF000000;
        var bgColor: UInt32 = 0;
        var weight: UInt16 = 400;
        var underline = false;
        var italic = false;
        if let fg = json["fg_color"] as? UInt32 {
            fgColor = fg;
        }
        if let bg = json["bg_color"] as? UInt32 {
            bgColor = bg;
        }
        if let w = json["weight"] as? UInt16 {
            weight = w;
        }
        if let u = json["underline"] as? Bool {
            underline = u;
        }
        if let i = json["italic"] as? Bool {
            italic = i;
        }
        let style = Style(fgColor: colorFromArgb(fgColor), bgColor: colorFromArgb(bgColor), weight: weight, underline: underline, italic: italic);
        while map.count < id {
            map.append(nil)
        }
        if map.count == id {
            map.append(style)
        } else {
            map[id] = style
        }
    }

    private func applyStyle(string: NSMutableAttributedString, id: Int, range: NSRange) {
        if id >= map.count { return }
        guard let style = map[id] else { return }
        string.addAttribute(NSForegroundColorAttributeName, value: style.fgColor, range: range)
        if style.bgColor.alphaComponent != 0.0 {
            string.addAttribute(NSBackgroundColorAttributeName, value: style.bgColor, range: range)
        }
        if style.underline {
            string.addAttribute(NSUnderlineStyleAttributeName, value: NSUnderlineStyle.styleSingle.rawValue, range: range)
        }
        if style.weight > 500 {
            // TODO: apply actual numeric weight
            string.applyFontTraits(NSFontTraitMask.boldFontMask, range: range)
        }
        if style.italic {
            // TODO: use true italic in font if available
            string.addAttribute(NSObliquenessAttributeName, value: 0.2, range: range)
        }
    }

    // Apply styles to the given string.
    // The selection color (for which style 0 is reserverd) is passed in, as it might be different for
    // different windows (while the StyleMap object is shared).
    func applyStyles(text: String, string: NSMutableAttributedString, styles: [StyleSpan], selColor: NSColor) {
        queue.sync {
            for styleSpan in styles {
                if styleSpan.style == 0 {
                    string.addAttribute(NSBackgroundColorAttributeName, value: selColor, range: styleSpan.range)
                } else {
                    applyStyle(string: string, id: styleSpan.style, range: styleSpan.range)
                }
            }
        }
    }
}
