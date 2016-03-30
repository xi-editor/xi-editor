// Copyright 2016 Google Inc. All rights reserved.
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

class EditView: NSView {

    var text: [[AnyObject]]
    
    var eventCallback: (NSEvent -> ())?

    var widthConstraint: NSLayoutConstraint?
    var heightConstraint: NSLayoutConstraint?

    var attributes: [String: AnyObject]
    var ascent: CGFloat
    var descent: CGFloat
    var leading: CGFloat
    var baseline: CGFloat
    var linespace: CGFloat

    let selcolor: NSColor

    override init(frame frameRect: NSRect) {
        let font = CTFontCreateWithName("InconsolataGo", 14, nil)
        ascent = CTFontGetAscent(font)
        descent = CTFontGetDescent(font)
        leading = CTFontGetLeading(font)
        linespace = ceil(ascent + descent + leading)
        baseline = ceil(ascent)
        attributes = [String(kCTFontAttributeName): font]
        text = []
        selcolor = NSColor(colorLiteralRed: 0.7, green: 0.85, blue: 0.99, alpha: 1.0)
        super.init(frame: frameRect)
        widthConstraint = NSLayoutConstraint(item: self, attribute: .Width, relatedBy: .GreaterThanOrEqual, toItem: nil, attribute: .Width, multiplier: 1, constant: 100)
        widthConstraint!.active = true
        heightConstraint = NSLayoutConstraint(item: self, attribute: .Height, relatedBy: .GreaterThanOrEqual, toItem: nil, attribute: .Height, multiplier: 1, constant: 100)
        heightConstraint!.active = true
    }

    required init?(coder: NSCoder) {
        fatalError("View doesn't support NSCoding")
    }

    func utf8_offset_to_utf16(s: String, _ ix: Int) -> Int {
        // String(s.utf8.prefix(ix)).utf16.count
        return s.utf8.startIndex.advancedBy(ix).samePositionIn(s.utf16)!._offset
    }

    override func drawRect(dirtyRect: NSRect) {
        super.drawRect(dirtyRect)
        let path = NSBezierPath(ovalInRect: frame)
        NSColor(colorLiteralRed: 0, green: 0, blue: 1, alpha: 0.25).setFill()
        path.fill()
        let path2 = NSBezierPath(ovalInRect: dirtyRect)
        NSColor(colorLiteralRed: 0, green: 0.5, blue: 0, alpha: 0.25).setFill()
        path2.fill()

        let context = NSGraphicsContext.currentContext()!.CGContext
        let x0: CGFloat = 2;
        var y = baseline;
        for line in text {
            let s = line[0] as! String
            let attrString = NSMutableAttributedString(string: s, attributes: self.attributes)
            var cursor: Int? = nil;
            for attr in line.dropFirst() {
                let attr = attr as! [AnyObject]
                let type = attr[0] as! String
                if type == "cursor" {
                    cursor = attr[1] as? Int
                } else if type == "sel" {
                    let start = attr[1] as! Int
                    let u16_start = utf8_offset_to_utf16(s, start)
                    let end = attr[2] as! Int
                    let u16_end = utf8_offset_to_utf16(s, end)
                    attrString.addAttribute(NSBackgroundColorAttributeName, value: selcolor, range: NSMakeRange(u16_start, u16_end - u16_start))
                }
            }
            // TODO: I don't understand where the 13 comes from (it's what aligns with baseline. We
            // probably want to move to using CTLineDraw instead of drawing the attributed string,
            // but that means drawing the selection highlight ourselves (which has other benefits).
            //attrString.drawAtPoint(NSPoint(x: x0, y: y - 13))
            attrString.drawWithRect(NSRect(x: x0, y: y, width: dirtyRect.width - x0, height: 14), options: [])
            if let cursor = cursor {
                let ctline = CTLineCreateWithAttributedString(attrString)
                /*
                CGContextSetTextMatrix(context, CGAffineTransform(a: 1, b: 0, c: 0, d: -1, tx: x0, ty: y))
                CTLineDraw(ctline, context)
                */
                let utf16_ix = utf8_offset_to_utf16(s, cursor)
                Swift.print(utf16_ix)
                let pos = CTLineGetOffsetForStringIndex(ctline, CFIndex(utf16_ix), nil)
                CGContextMoveToPoint(context, x0 + pos, y + descent)
                CGContextAddLineToPoint(context, x0 + pos, y - ascent)
                CGContextStrokePath(context)
            }
            y += linespace
        }
    }

    override var acceptsFirstResponder: Bool {
        return true;
    }

    // we use a flipped coordinate system primarily to get better alignment when scrolling
    override var flipped: Bool {
        return true;
    }

    override func resizeWithOldSuperviewSize(oldSize: NSSize) {
        super.resizeWithOldSuperviewSize(oldSize)
        Swift.print("resizing, oldsize =", oldSize, ", frame =", frame);
    }

    override func keyDown(theEvent: NSEvent) {
        if let callback = eventCallback {
            callback(theEvent)
        } else {
            super.keyDown(theEvent)
        }
    }

    func mySetText(text: [[AnyObject]]) {
        self.text = text
        //heightConstraint?.constant = 400
        needsDisplay = true
    }

}
