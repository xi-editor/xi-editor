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
class StyleMap {
    private var map: [Style?] = []

    func defStyle(json: [String: AnyObject]) {
        guard let styleID = json["id"] as? Int else { return }
        let fgColor = json["fg_color"] as? UInt32 ?? 0xFF000000
        let bgColor = json["bg_color"] as? UInt32 ?? 0
        let weight = json["weight"] as? UInt16 ?? 400
        let underline = json["underline"] as? Bool ?? false
        let italic = json["italic"] as? Bool ?? false
        
        let style = Style(fgColor: colorFromArgb(fgColor), bgColor: colorFromArgb(bgColor), weight: weight, underline: underline, italic: italic);
        while map.count < styleID {
            map.append(nil)
        }
        if map.count == styleID {
            map.append(style)
        } else {
            map[styleID] = style
        }
    }

    /// applies a given style to the AttributedString
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

    /// Given style information, applies the appropriate text attributes to the passed NSAttributedString
    func applyStyles(text: String, string: inout NSMutableAttributedString, styles: [StyleSpan]) {
        // we handle the 0 style (selection) in EditView.drawRect
        for styleSpan in styles.filter({ $0.style != 0 }) {
                applyStyle(string: string, id: styleSpan.style, range: styleSpan.range)
        }
    }
}
