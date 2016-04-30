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

func eventToJson(event: NSEvent) -> AnyObject {
    let flags = event.modifierFlags.rawValue >> 16;
    return ["key", ["keycode": Int(event.keyCode),
        "chars": event.characters!,
        "flags": flags]]
}

// compute the width if monospaced, 0 otherwise
func getFontWidth(font: CTFont) -> CGFloat {
    if (font as NSFont).fixedPitch {
        let characters = [UniChar(0x20)]
        var glyphs = [CGGlyph(0)]
        if CTFontGetGlyphsForCharacters(font, characters, &glyphs, 1) {
            let advance = CTFontGetAdvancesForGlyphs(font, .Horizontal, glyphs, nil, 1)
            return CGFloat(advance)
        }
    }
    return 0
}

func colorFromArgb(argb: UInt32) -> NSColor {
    return NSColor(red: CGFloat((argb >> 16) & 0xff) * 1.0/255,
        green: CGFloat((argb >> 8) & 0xff) * 1.0/255,
        blue: CGFloat(argb & 0xff) * 1.0/255,
        alpha: CGFloat((argb >> 24) & 0xff) * 1.0/255)
}

class EditView: NSView {

    // basically a cache of lines, indexed by line number
    var lineMap: [Int: [AnyObject]] = [:]
    var height: Int = 0

    var coreConnection: CoreConnection?

    var widthConstraint: NSLayoutConstraint?
    var heightConstraint: NSLayoutConstraint?

    var attributes: [String: AnyObject]
    var ascent: CGFloat
    var descent: CGFloat
    var leading: CGFloat
    var baseline: CGFloat
    var linespace: CGFloat
    var fontWidth: CGFloat

    let selcolor: NSColor

    // visible scroll region, exclusive of lastLine
    var firstLine: Int = 0
    var lastLine: Int = 0

    // magic for accepting updates from other threads
    var updateQueue: dispatch_queue_t
    var pendingUpdate: [String: AnyObject]? = nil

    override init(frame frameRect: NSRect) {
        let font = CTFontCreateWithName("InconsolataGo", 14, nil)
        ascent = CTFontGetAscent(font)
        descent = CTFontGetDescent(font)
        leading = CTFontGetLeading(font)
        linespace = ceil(ascent + descent + leading)
        baseline = ceil(ascent)
        attributes = [String(kCTFontAttributeName): font]
        fontWidth = getFontWidth(font)
        selcolor = NSColor(colorLiteralRed: 0.7, green: 0.85, blue: 0.99, alpha: 1.0)
        updateQueue = dispatch_queue_create("com.levien.xi.update", DISPATCH_QUEUE_SERIAL)
        super.init(frame: frameRect)
        widthConstraint = NSLayoutConstraint(item: self, attribute: .Width, relatedBy: .GreaterThanOrEqual, toItem: nil, attribute: .Width, multiplier: 1, constant: 400)
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
        /*
        let path = NSBezierPath(ovalInRect: frame)
        NSColor(colorLiteralRed: 0, green: 0, blue: 1, alpha: 0.25).setFill()
        path.fill()
        let path2 = NSBezierPath(ovalInRect: dirtyRect)
        NSColor(colorLiteralRed: 0, green: 0.5, blue: 0, alpha: 0.25).setFill()
        path2.fill()
        */

        let context = NSGraphicsContext.currentContext()!.CGContext
        let x0: CGFloat = 2;
        let first = Int(floor(dirtyRect.origin.y / linespace))
        let last = Int(ceil((dirtyRect.origin.y + dirtyRect.size.height) / linespace))

        for lineIx in first..<last {
            let line = getLine(lineIx)
            if line == nil {
                continue
            }
            let s = line![0] as! String
            let attrString = NSMutableAttributedString(string: s, attributes: self.attributes)
            /*
            let randcolor = NSColor(colorLiteralRed: Float(drand48()), green: Float(drand48()), blue: Float(drand48()), alpha: 1.0)
            attrString.addAttribute(NSForegroundColorAttributeName, value: randcolor, range: NSMakeRange(0, s.utf16.count))
            */
            var cursor: Int? = nil;
            for attr in line!.dropFirst() {
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
                } else if type == "fg" {
                    let start = attr[1] as! Int
                    let u16_start = utf8_offset_to_utf16(s, start)
                    let end = attr[2] as! Int
                    let u16_end = utf8_offset_to_utf16(s, end)
                    let fgcolor = colorFromArgb(UInt32(attr[3] as! Int))
                    //let fgcolor = colorFromArgb(0xff800000)
                    attrString.addAttribute(NSForegroundColorAttributeName, value: fgcolor, range: NSMakeRange(u16_start, u16_end - u16_start))
                }
            }
            // TODO: I don't understand where the 13 comes from (it's what aligns with baseline. We
            // probably want to move to using CTLineDraw instead of drawing the attributed string,
            // but that means drawing the selection highlight ourselves (which has other benefits).
            //attrString.drawAtPoint(NSPoint(x: x0, y: y - 13))
            let y = linespace * CGFloat(lineIx + 1);
            attrString.drawWithRect(NSRect(x: x0, y: y, width: dirtyRect.origin.x + dirtyRect.width - x0, height: 14), options: [])
            if let cursor = cursor {
                let ctline = CTLineCreateWithAttributedString(attrString)
                /*
                CGContextSetTextMatrix(context, CGAffineTransform(a: 1, b: 0, c: 0, d: -1, tx: x0, ty: y))
                CTLineDraw(ctline, context)
                */
                var pos = CGFloat(0)
                // special case because measurement is so expensive; might have to rethink in rtl
                if cursor != 0 {
                    let utf16_ix = utf8_offset_to_utf16(s, cursor)
                    pos = CTLineGetOffsetForStringIndex(ctline, CFIndex(utf16_ix), nil)
                }
                CGContextSetStrokeColorWithColor(context, CGColorCreateGenericGray(0, 1))
                CGContextMoveToPoint(context, x0 + pos, y + descent)
                CGContextAddLineToPoint(context, x0 + pos, y - ascent)
                CGContextStrokePath(context)
            }
        }
    }

    override var acceptsFirstResponder: Bool {
        return true;
    }

    // we use a flipped coordinate system primarily to get better alignment when scrolling
    override var flipped: Bool {
        return true;
    }

    // TODO: probably get rid of this, we get scroll notifications elsewhere
    override func resizeWithOldSuperviewSize(oldSize: NSSize) {
        super.resizeWithOldSuperviewSize(oldSize)
        //Swift.print("resizing, oldsize =", oldSize, ", frame =", frame);
    }

    override func keyDown(theEvent: NSEvent) {
        if let coreConnection = coreConnection {
            coreConnection.sendJson(eventToJson(theEvent))
        } else {
            super.keyDown(theEvent)
        }
    }

    func updateText(text: [String: AnyObject]) {
        self.lineMap = [:]
        let firstLine = text["first_line"] as! Int
        self.height = text["height"] as! Int
        let lines = text["lines"]! as! [[AnyObject]]
        for lineNum in firstLine..<(firstLine + lines.count) {
            self.lineMap[lineNum] = lines[lineNum - firstLine]
        }
        heightConstraint?.constant = CGFloat(self.height) * linespace + 2 * descent
        if let cursor = text["scrollto"] as? [Int] {
            let line = cursor[0]
            let col = cursor[1]
            let x = CGFloat(col) * fontWidth  // TODO: deal with non-ASCII, non-monospaced case
            let y = CGFloat(line) * linespace + baseline
            let scrollRect = NSRect(x: x, y: y - baseline, width: 4, height: linespace + descent)
            dispatch_async(dispatch_get_main_queue()) {
                // defer until resize has had a chance to happen
                self.scrollRectToVisible(scrollRect)
            }
        }
        needsDisplay = true
    }

    func tryUpdate() {
        var pendingUpdate: [String: AnyObject]?
        dispatch_sync(updateQueue) {
            pendingUpdate = self.pendingUpdate
            self.pendingUpdate = nil
        }
        if let text = pendingUpdate {
            updateText(text)
        }
    }

    func updateSafe(text: [String: AnyObject]) {
        dispatch_sync(updateQueue) {
            self.pendingUpdate = text
        }
        dispatch_async(dispatch_get_main_queue()) {
            self.tryUpdate()
        }
    }

    func updateScroll(bounds: NSRect) {
        let first = Int(floor(bounds.origin.y / linespace))
        let height = Int(ceil((bounds.size.height) / linespace))
        let last = first + height
        if first != firstLine || last != lastLine {
            firstLine = first
            lastLine = last
            coreConnection?.sendJson(["scroll", [firstLine, lastLine]])
        }
    }

    let MAX_CACHE_LINES = 1000
    let CACHE_FETCH_CHUNK = 100

    // get a line, trying to hit the cache
    func getLine(lineNum: Int) -> [AnyObject]? {
        // TODO: maybe core should take care to set height >= 1
        if lineNum < 0 || lineNum >= max(1, self.height) {
            return nil
        }
        if let line = self.lineMap[lineNum] {
            return line
        }
        // speculatively prefetch a bigger chunk from RPC, but don't get anything we already have
        var first = lineNum
        while first > max(0, lineNum - CACHE_FETCH_CHUNK) && lineMap.indexForKey(first - 1) == nil {
            first -= 1
        }
        var last = lineNum + 1
        while last < min(lineNum, height) + CACHE_FETCH_CHUNK && lineMap.indexForKey(last + 1) == nil {
            last += 1
        }
        if lineMap.count + (last - first) > MAX_CACHE_LINES {
            // a more sophisticated approach would be LRU replacement, but simple is probably good enough
            lineMap = [:]
        }
        if let lines = fetchLineRange(first, last) {
            for lineNum in first..<last {
                if lineNum - first < lines.count {
                    lineMap[lineNum] = lines[lineNum - first]
                } else {
                    lineMap[lineNum] = [""]  // TODO: maybe core should always supply
                }
            }
        }
        return lineMap[lineNum]
    }
    
    func fetchLineRange(first: Int, _ last: Int) -> [[AnyObject]]? {
        let start = NSDate()
        if let result = coreConnection?.sendRpc(["render_lines", ["first_line": first, "last_line": last]]) as? [[AnyObject]] {
            let interval = NSDate().timeIntervalSinceDate(start)
            Swift.print(String(format: "RPC latency = %3.2fms", interval as Double * 1e3))
            return result
        } else {
            Swift.print("rpc error")
            return nil
        }
    }
}
