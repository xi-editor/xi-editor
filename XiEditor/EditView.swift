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
    return ["keycode": Int(event.keyCode),
        "chars": event.characters!,
        "flags": flags]
}

func insertedStringToJson(stringToInsert: NSString) -> AnyObject {
    return ["chars": stringToInsert]
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

func camelCaseToUnderscored(name: NSString) -> NSString {
    let underscored = NSMutableString();
    let scanner = NSScanner(string: name as String);
    let notUpperCase = NSCharacterSet.uppercaseLetterCharacterSet().invertedSet;
    var notUpperCaseFragment: NSString?
    while (scanner.scanUpToCharactersFromSet(NSCharacterSet.uppercaseLetterCharacterSet(), intoString: &notUpperCaseFragment)) {
        underscored.appendString(notUpperCaseFragment! as String);
        var upperCaseFragement: NSString?
        if (scanner.scanUpToCharactersFromSet(notUpperCase, intoString: &upperCaseFragement)) {
            underscored.appendString("_");
            let downcasedFragment = upperCaseFragement!.lowercaseString;
            underscored.appendString(downcasedFragment);
        }
    }
    return underscored;
}

class EditView: NSView, NSTextInputClient {
    var tabName: String?
    var coreConnection: CoreConnection?

    // basically a cache of lines, indexed by line number
    var lineMap: [Int: [AnyObject]] = [:]
    var height: Int = 0

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
    
    var lastDragLineCol: (Int, Int)?
    var timer: NSTimer?
    var timerEvent: NSEvent?

    // magic for accepting updates from other threads
    var updateQueue: dispatch_queue_t
    var pendingUpdate: [String: AnyObject]? = nil

    var currentEvent: NSEvent?
    
    var cursorPos: (Int, Int)?
    var _selectedRange: NSRange
    var _markedRange: NSRange

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
        _selectedRange = NSMakeRange(NSNotFound, 0)
        _markedRange = NSMakeRange(NSNotFound, 0)
        super.init(frame: frameRect)
        widthConstraint = NSLayoutConstraint(item: self, attribute: .Width, relatedBy: .GreaterThanOrEqual, toItem: nil, attribute: .Width, multiplier: 1, constant: 400)
        widthConstraint!.active = true
        heightConstraint = NSLayoutConstraint(item: self, attribute: .Height, relatedBy: .GreaterThanOrEqual, toItem: nil, attribute: .Height, multiplier: 1, constant: 100)
        heightConstraint!.active = true
    }

    required init?(coder: NSCoder) {
        fatalError("View doesn't support NSCoding")
    }

    func sendRpcAsync(method: String, params: AnyObject) {
        let inner = ["method": method, "params": params, "tab": tabName!] as [String : AnyObject]
        coreConnection?.sendRpcAsync("edit", params: inner)
    }

    func sendRpc(method: String, params: AnyObject) -> AnyObject? {
        let inner = ["method": method, "params": params, "tab": tabName!] as [String : AnyObject]
        return coreConnection?.sendRpc("edit", params: inner)
    }

    func utf8_offset_to_utf16(s: String, _ ix: Int) -> Int {
        // String(s.utf8.prefix(ix)).utf16.count
        return s.utf8.startIndex.advancedBy(ix).samePositionIn(s.utf16)!._offset
    }

    func utf16_offset_to_utf8(s: String, _ ix: Int) -> Int {
        return String(s.utf16.prefix(ix)).utf8.count
    }

    let x0: CGFloat = 2;

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
                    self.cursorPos = (lineIx, utf8_offset_to_utf16(s, cursor!))
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
            if let c = cursor {
                let cix = utf8_offset_to_utf16(s, c)
                if (markedRange().location != NSNotFound) {
                    let markRangeStart = cix - markedRange().length
                    if (markRangeStart >= 0) {
                        attrString.addAttribute(NSUnderlineStyleAttributeName,
                                                value: NSUnderlineStyle.StyleSingle.rawValue,
                                                range: NSMakeRange(markRangeStart, markedRange().length))
                    }
                }
                if (selectedRange().location != NSNotFound) {
                    let selectedRangeStart = cix - markedRange().length + selectedRange().location
                    if (selectedRangeStart >= 0) {
                        attrString.addAttribute(NSUnderlineStyleAttributeName,
                                                value: NSUnderlineStyle.StyleThick.rawValue,
                                                range: NSMakeRange(selectedRangeStart, selectedRange().length))
                    }
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
        // store current event so that it can be sent to the core
        // if the selector for the event is "noop:".
        currentEvent = theEvent;
        self.inputContext?.handleEvent(theEvent);
        currentEvent = nil;
    }

    // NSResponder (used mostly for paste)
    override func insertText(insertString: AnyObject) {
        sendRpcAsync("insert", params: insertedStringToJson(insertString as! NSString))
    }

    // NSTextInputClient protocol
    func insertText(aString: AnyObject, replacementRange: NSRange) {
        self.removeMarkedText()
        self.replaceCharactersInRange(replacementRange, withText: aString)
    }
    
    func replacementMarkedRange(replacementRange: NSRange) -> NSRange {
        var markedRange = _markedRange
        
        
        if (markedRange.location == NSNotFound) {
            markedRange = _selectedRange
        }
        if (replacementRange.location != NSNotFound) {
            var newRange: NSRange = markedRange
            newRange.location += replacementRange.location
            newRange.length += replacementRange.length
            if (NSMaxRange(newRange) <= NSMaxRange(markedRange)) {
                markedRange = newRange
            }
        }
        
        return markedRange
    }
    
    func replaceCharactersInRange(aRange: NSRange, withText aString: AnyObject) -> NSRange {
        var replacementRange = aRange
        var len = 0
        if let attrStr = aString as? NSAttributedString {
            len = attrStr.string.characters.count
        } else if let str = aString as? NSString {
            len = str.length
        }
        if (replacementRange.location == NSNotFound) {
            replacementRange.location = 0
            replacementRange.length = 0
        }
        for _ in 0..<aRange.length {
            sendRpcAsync("delete_backward", params: [])
        }
        if let attrStr = aString as? NSAttributedString {
            sendRpcAsync("insert", params: insertedStringToJson(attrStr.string))
        } else if let str = aString as? NSString {
            sendRpcAsync("insert", params: insertedStringToJson(str))
        }
        return NSMakeRange(replacementRange.location, len)
    }
    
    func setMarkedText(aString: AnyObject, selectedRange: NSRange, replacementRange: NSRange) {
        var mutSelectedRange = selectedRange
        let effectiveRange = self.replaceCharactersInRange(self.replacementMarkedRange(replacementRange), withText: aString)
        if (selectedRange.location != NSNotFound) {
            mutSelectedRange.location += effectiveRange.location
        }
        _selectedRange = mutSelectedRange
        _markedRange = effectiveRange
        if (effectiveRange.length == 0) {
            self.removeMarkedText()
        }
    }
    
    func removeMarkedText() {
        if (_markedRange.location != NSNotFound) {
            for _ in 0..<_markedRange.length {
                sendRpcAsync("delete_backward", params: [])
            }
        }
        _markedRange = NSMakeRange(NSNotFound, 0)
        _selectedRange = NSMakeRange(NSNotFound, 0)
    }
    
    func unmarkText() {
        self._markedRange = NSMakeRange(NSNotFound, 0)
    }
    
    func selectedRange() -> NSRange {
        return _selectedRange
    }
    
    func markedRange() -> NSRange {
        return _markedRange
    }
    
    func hasMarkedText() -> Bool {
        return _markedRange.location != NSNotFound
    }
    
    func attributedSubstringForProposedRange(aRange: NSRange, actualRange: NSRangePointer) -> NSAttributedString? {
        return NSAttributedString()
    }
    
    func validAttributesForMarkedText() -> [String] {
        return [NSForegroundColorAttributeName, NSBackgroundColorAttributeName]
    }
    
    func firstRectForCharacterRange(aRange: NSRange, actualRange: NSRangePointer) -> NSRect {
        if let viewWinFrame = self.window?.convertRectToScreen(self.frame),
            let (lineIx, pos) = self.cursorPos,
            let line = getLine(lineIx) {
            let str = line[0] as! String
            let ctLine = CTLineCreateWithAttributedString(NSMutableAttributedString(string: str, attributes: self.attributes))
            let rangeWidth = CTLineGetOffsetForStringIndex(ctLine, pos, nil) - CTLineGetOffsetForStringIndex(ctLine, pos - aRange.length, nil)
            return NSRect(x: viewWinFrame.origin.x + CTLineGetOffsetForStringIndex(ctLine, pos, nil),
                          y: viewWinFrame.origin.y + viewWinFrame.size.height - linespace * CGFloat(lineIx + 1) - 5,
                          width: rangeWidth,
                          height: linespace)
        } else {
            return NSRect(x: 0, y: 0, width: 0, height: 0)
        }
    }
    
    func characterIndexForPoint(aPoint: NSPoint) -> Int {
        return 0
    }
    
    override func doCommandBySelector(aSelector: Selector) {
        if (self.respondsToSelector(aSelector)) {
            super.doCommandBySelector(aSelector);
        } else {
            let commandName = camelCaseToUnderscored(aSelector.description).stringByReplacingOccurrencesOfString(":", withString: "");
            if (commandName == "noop") {
                sendRpcAsync("key", params: eventToJson(currentEvent!));
            } else {
                sendRpcAsync(commandName, params: []);
            }
        }
    }

    func cutCopy(method: String) {
        let text = sendRpc(method, params: [])
        if let text = text as? String {
            let pasteboard = NSPasteboard.generalPasteboard()
            pasteboard.clearContents()
            pasteboard.writeObjects([text])
        }
    }

    func cut(sender: AnyObject?) {
        cutCopy("cut")
    }

    func copy(sender: AnyObject?) {
        cutCopy("copy")
    }

    func paste(sender: AnyObject?) {
        let pasteboard = NSPasteboard.generalPasteboard()
        if let items = pasteboard.pasteboardItems {
            for element in items {
                if let str = element.stringForType("public.utf8-plain-text") {
                    insertText(str)
                    break
                }
            }
        }
    }

    func undo(sender: AnyObject?) {
        sendRpcAsync("undo", params: [])
    }

    func redo(sender: AnyObject?) {
        sendRpcAsync("redo", params: [])
    }

    override func mouseDown(theEvent: NSEvent) {
        removeMarkedText()
        self.inputContext?.discardMarkedText()
        let (line, col) = pointToLineCol(convertPoint(theEvent.locationInWindow, fromView: nil))
        lastDragLineCol = (line, col)
        let flags = theEvent.modifierFlags.rawValue >> 16
        let clickCount = theEvent.clickCount
        sendRpcAsync("click", params: [line, col, flags, clickCount])
        timer = NSTimer.scheduledTimerWithTimeInterval(NSTimeInterval(1.0/60), target: self, selector: #selector(autoscrollTimer), userInfo: nil, repeats: true)
        timerEvent = theEvent
    }
    
    override func mouseDragged(theEvent: NSEvent) {
        autoscroll(theEvent)
        let (line, col) = pointToLineCol(convertPoint(theEvent.locationInWindow, fromView: nil))
        if let last = lastDragLineCol where last != (line, col) {
            lastDragLineCol = (line, col)
            let flags = theEvent.modifierFlags.rawValue >> 16
            sendRpcAsync("drag", params: [line, col, flags])
        }
        timerEvent = theEvent
    }

    override func mouseUp(theEvent: NSEvent) {
        timer?.invalidate()
        timer = nil
        timerEvent = nil
    }

    func autoscrollTimer() {
        if let event = timerEvent {
            mouseDragged(event)
        }
    }

    // TODO: more functions should call this, just dividing by linespace doesn't account for descent
    func yToLine(y: CGFloat) -> Int {
        return Int(floor(max(y - descent, 0) / linespace))
    }

    func lineIxToBaseline(lineIx: Int) -> CGFloat {
        return CGFloat(lineIx + 1) * linespace
    }

    func pointToLineCol(loc: NSPoint) -> (Int, Int) {
        let lineIx = yToLine(loc.y)
        var col = 0
        if let line = getLine(lineIx) {
            let s = line[0] as! String
            let attrString = NSAttributedString(string: s, attributes: self.attributes)
            let ctline = CTLineCreateWithAttributedString(attrString)
            let relPos = NSPoint(x: loc.x - x0, y: lineIxToBaseline(lineIx) - loc.y)
            let utf16_ix = CTLineGetStringIndexForPosition(ctline, relPos)
            if utf16_ix != kCFNotFound {
                col = utf16_offset_to_utf8(s, utf16_ix)
            }
        }
        return (lineIx, col)
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
            sendRpcAsync("scroll", params: [firstLine, lastLine])
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
        if let result = sendRpc("render_lines", params: ["first_line": first, "last_line": last]) as? [[AnyObject]] {
            let interval = NSDate().timeIntervalSinceDate(start)
            Swift.print(String(format: "RPC latency = %3.2fms", interval as Double * 1e3))
            return result
        } else {
            Swift.print("rpc error")
            return nil
        }
    }

    // MARK: - Debug Methods

    @IBAction func debugRewrap(sender: AnyObject) {
        sendRpcAsync("debug_rewrap", params: [])
    }

    @IBAction func debugTestFGSpans(sender: AnyObject) {
        sendRpcAsync("debug_test_fg_spans", params: [])
    }

    @IBAction func debugRunPlugin(sender: AnyObject) {
        sendRpcAsync("debug_run_plugin", params: [])
    }
}
