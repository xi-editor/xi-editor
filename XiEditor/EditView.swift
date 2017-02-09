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

func eventToJson(_ event: NSEvent) -> Any {
    let flags = event.modifierFlags.rawValue >> 16;
    return ["keycode": Int(event.keyCode),
        "chars": event.characters!,
        "flags": flags]
}

func insertedStringToJson(_ stringToInsert: NSString) -> Any {
    return ["chars": stringToInsert]
}

// compute the width if monospaced, 0 otherwise
func getFontWidth(_ font: CTFont) -> CGFloat {
    if (font as NSFont).isFixedPitch {
        let characters = [UniChar(0x20)]
        var glyphs = [CGGlyph(0)]
        if CTFontGetGlyphsForCharacters(font, characters, &glyphs, 1) {
            let advance = CTFontGetAdvancesForGlyphs(font, .horizontal, glyphs, nil, 1)
            return CGFloat(advance)
        }
    }
    return 0
}

func colorFromArgb(_ argb: UInt32) -> NSColor {
    return NSColor(red: CGFloat((argb >> 16) & 0xff) * 1.0/255,
        green: CGFloat((argb >> 8) & 0xff) * 1.0/255,
        blue: CGFloat(argb & 0xff) * 1.0/255,
        alpha: CGFloat((argb >> 24) & 0xff) * 1.0/255)
}

func camelCaseToUnderscored(_ name: NSString) -> NSString {
    let underscored = NSMutableString();
    let scanner = Scanner(string: name as String);
    let notUpperCase = CharacterSet.uppercaseLetters.inverted;
    var notUpperCaseFragment: NSString?
    while (scanner.scanUpToCharacters(from: CharacterSet.uppercaseLetters, into: &notUpperCaseFragment)) {
        underscored.append(notUpperCaseFragment! as String);
        var upperCaseFragement: NSString?
        if (scanner.scanUpToCharacters(from: notUpperCase, into: &upperCaseFragement)) {
            underscored.append("_");
            let downcasedFragment = upperCaseFragement!.lowercased;
            underscored.append(downcasedFragment);
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

    let fgSelcolor: NSColor
    let bgSelcolor: NSColor

    // visible scroll region, exclusive of lastLine
    var firstLine: Int = 0
    var lastLine: Int = 0
    
    var lastDragLineCol: (Int, Int)?
    var timer: Timer?
    var timerEvent: NSEvent?

    // magic for accepting updates from other threads
    var updateQueue: DispatchQueue
    var pendingUpdate: [String: AnyObject]? = nil

    var currentEvent: NSEvent?
    
    var cursorPos: (Int, Int)?
    var _selectedRange: NSRange
    var _markedRange: NSRange
    
    var frameRect: NSRect
    
    var isFrontmost: Bool // Are we frontmost, the view that gets keyboard input?
    
    var cursorFlashOn: Bool
    var blinkTimer : Timer?

    override init(frame frameRect: NSRect) {
        let font = CTFontCreateWithName("InconsolataGo" as CFString?, 14, nil)
        ascent = CTFontGetAscent(font)
        descent = CTFontGetDescent(font)
        leading = CTFontGetLeading(font)
        linespace = ceil(ascent + descent + leading)
        baseline = ceil(ascent)
        attributes = [String(kCTFontAttributeName): font]
        fontWidth = getFontWidth(font)
        fgSelcolor =  NSColor.selectedTextBackgroundColor
        bgSelcolor = NSColor(colorLiteralRed: 0.8, green: 0.8, blue: 0.8, alpha: 1.0) //Gray for the selected text background when not 'key'
        updateQueue = DispatchQueue(label: "com.levien.xi.update", attributes: [])
        _selectedRange = NSMakeRange(NSNotFound, 0)
        _markedRange = NSMakeRange(NSNotFound, 0)
        self.frameRect = frameRect
        isFrontmost = false
        cursorFlashOn = true
        super.init(frame: frameRect)
        widthConstraint = NSLayoutConstraint(item: self, attribute: .width, relatedBy: .greaterThanOrEqual, toItem: nil, attribute: .width, multiplier: 1, constant: 400)
        widthConstraint!.isActive = true
        heightConstraint = NSLayoutConstraint(item: self, attribute: .height, relatedBy: .greaterThanOrEqual, toItem: nil, attribute: .height, multiplier: 1, constant: 100)
        heightConstraint!.isActive = true
    }
    
    override func changeFont(_ sender: Any?) {
        Swift.print("changeFont...")
        let oldFont = attributes[String(kCTFontAttributeName)] as! CTFont
        let font = (sender as! NSFontManager).convert(oldFont)
        ascent = CTFontGetAscent(font)
        descent = CTFontGetDescent(font)
        leading = CTFontGetLeading(font)
        linespace = ceil(ascent + descent + leading)
        baseline = ceil(ascent)
        attributes[String(kCTFontAttributeName)] = font
        fontWidth = getFontWidth(font)
        needsDisplay = true
    }

    required init?(coder: NSCoder) {
        fatalError("View doesn't support NSCoding")
    }

    func sendRpcAsync(_ method: String, params: Any) {
        let inner = ["method": method as AnyObject, "params": params, "tab": tabName! as AnyObject] as [String : Any]
        coreConnection?.sendRpcAsync("edit", params: inner)
    }

    func sendRpc(_ method: String, params: Any) -> Any? {
        let inner = ["method": method as AnyObject, "params": params, "tab": tabName! as AnyObject] as [String : Any]
        return coreConnection?.sendRpc("edit", params: inner)
    }

    func utf8_offset_to_utf16(_ s: String, _ ix: Int) -> Int {
        // String(s.utf8.prefix(ix)).utf16.count
        return s.utf8.index(s.utf8.startIndex, offsetBy: ix).samePosition(in: s.utf16)!._offset
    }

    func utf16_offset_to_utf8(_ s: String, _ ix: Int) -> Int {
        return String(describing: s.utf16.prefix(ix)).utf8.count
    }

    let x0: CGFloat = 2;

    let font_style_bold: Int = 1;
    let font_style_underline: Int = 2;
    let font_style_italic: Int = 4;

    override func draw(_ dirtyRect: NSRect) {
        if tabName == nil { return }
        super.draw(dirtyRect)
        /*
        let path = NSBezierPath(ovalInRect: frame)
        NSColor(colorLiteralRed: 0, green: 0, blue: 1, alpha: 0.25).setFill()
        path.fill()
        let path2 = NSBezierPath(ovalInRect: dirtyRect)
        NSColor(colorLiteralRed: 0, green: 0.5, blue: 0, alpha: 0.25).setFill()
        path2.fill()
        */

        let context = NSGraphicsContext.current()!.cgContext
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
                    attrString.addAttribute(NSBackgroundColorAttributeName, value: selcolor(), range: NSMakeRange(u16_start, u16_end - u16_start))
                } else if type == "fg" {
                    let start = attr[1] as! Int
                    let u16_start = utf8_offset_to_utf16(s, start)
                    let end = attr[2] as! Int
                    let u16_end = utf8_offset_to_utf16(s, end)
                    let fgcolor = colorFromArgb(UInt32(attr[3] as! Int))
                    let font_style = attr[4] as! Int
                    //let fgcolor = colorFromArgb(0xff800000)
                    attrString.addAttribute(NSForegroundColorAttributeName, value: fgcolor, range: NSMakeRange(u16_start, u16_end - u16_start))
                    if (font_style & font_style_underline) != 0 {
                        attrString.addAttribute(NSUnderlineStyleAttributeName,
                                                value: NSUnderlineStyle.styleSingle.rawValue,
                                                range: NSMakeRange(u16_start, u16_end - u16_start))
                    }
                    let fake_italic = true  // TODO: figure this out based on font support
                    if fake_italic  && (font_style & font_style_italic) != 0 {
                        attrString.addAttribute(NSObliquenessAttributeName,
                                                value: 0.2,
                                                range: NSMakeRange(u16_start, u16_end - u16_start))
                    }
                    let trait_mask = font_style_bold | (fake_italic ? 0 : font_style_italic)
                    if (font_style & trait_mask) != 0 {
                        var traits: NSFontTraitMask
                        switch font_style & trait_mask {
                        case font_style_bold:
                            traits = NSFontTraitMask.boldFontMask
                        case font_style_italic:
                            traits = NSFontTraitMask.italicFontMask
                        case (font_style_bold | font_style_italic):
                            traits = [NSFontTraitMask.boldFontMask, NSFontTraitMask.italicFontMask]
                        default:
                            traits = []
                        }
                        attrString.applyFontTraits(traits, range: NSMakeRange(u16_start, u16_end - u16_start))
                    }
                }
            }
            if let c = cursor {
                let cix = utf8_offset_to_utf16(s, c)
                if (markedRange().location != NSNotFound) {
                    let markRangeStart = cix - markedRange().length
                    if (markRangeStart >= 0) {
                        attrString.addAttribute(NSUnderlineStyleAttributeName,
                                                value: NSUnderlineStyle.styleSingle.rawValue,
                                                range: NSMakeRange(markRangeStart, markedRange().length))
                    }
                }
                if (selectedRange().location != NSNotFound) {
                    let selectedRangeStart = cix - markedRange().length + selectedRange().location
                    if (selectedRangeStart >= 0) {
                        attrString.addAttribute(NSUnderlineStyleAttributeName,
                                                value: NSUnderlineStyle.styleThick.rawValue,
                                                range: NSMakeRange(selectedRangeStart, selectedRange().length))
                    }
                }
            }
            
            // TODO: I don't understand where the 13 comes from (it's what aligns with baseline. We
            // probably want to move to using CTLineDraw instead of drawing the attributed string,
            // but that means drawing the selection highlight ourselves (which has other benefits).
            //attrString.drawAtPoint(NSPoint(x: x0, y: y - 13))
            let y = linespace * CGFloat(lineIx + 1);
            attrString.draw(with: NSRect(x: x0, y: y, width: dirtyRect.origin.x + dirtyRect.width - x0, height: 14), options: [])
            if isFrontmost, let cursor = cursor {
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
                context.setStrokeColor(cursorColor())
                context.move(to: CGPoint(x: x0 + pos, y: y + descent))
                context.addLine(to: CGPoint(x: x0 + pos, y: y - ascent))
                context.strokePath()
            }
        }
    }
    
    override var acceptsFirstResponder: Bool {
        return true;
    }

    // we use a flipped coordinate system primarily to get better alignment when scrolling
    override var isFlipped: Bool {
        return true;
    }

    // TODO: probably get rid of this, we get scroll notifications elsewhere
    override func resize(withOldSuperviewSize oldSize: NSSize) {
        super.resize(withOldSuperviewSize: oldSize)
        //Swift.print("resizing, oldsize =", oldSize, ", frame =", frame);
    }

    override func keyDown(with theEvent: NSEvent) {
        // store current event so that it can be sent to the core
        // if the selector for the event is "noop:".
        currentEvent = theEvent;
        self.inputContext?.handleEvent(theEvent);
        currentEvent = nil;
    }

    // NSResponder (used mostly for paste)
    override func insertText(_ insertString: Any) {
        sendRpcAsync("insert", params: insertedStringToJson(insertString as! NSString))
    }

    // NSTextInputClient protocol
    func insertText(_ aString: Any, replacementRange: NSRange) {
        self.removeMarkedText()
        self.replaceCharactersInRange(replacementRange, withText: aString as AnyObject)
    }
    
    func replacementMarkedRange(_ replacementRange: NSRange) -> NSRange {
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
    
    func replaceCharactersInRange(_ aRange: NSRange, withText aString: AnyObject) -> NSRange {
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
            sendRpcAsync("delete_backward", params  : [])
        }
        if let attrStr = aString as? NSAttributedString {
            sendRpcAsync("insert", params: insertedStringToJson(attrStr.string as NSString))
        } else if let str = aString as? NSString {
            sendRpcAsync("insert", params: insertedStringToJson(str))
        }
        return NSMakeRange(replacementRange.location, len)
    }
    
    func setMarkedText(_ aString: Any, selectedRange: NSRange, replacementRange: NSRange) {
        var mutSelectedRange = selectedRange
        let effectiveRange = self.replaceCharactersInRange(self.replacementMarkedRange(replacementRange), withText: aString as AnyObject)
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
    
    func attributedSubstring(forProposedRange aRange: NSRange, actualRange: NSRangePointer?) -> NSAttributedString? {
        return NSAttributedString()
    }
    
    func validAttributesForMarkedText() -> [String] {
        return [NSForegroundColorAttributeName, NSBackgroundColorAttributeName]
    }
    
    func firstRect(forCharacterRange aRange: NSRange, actualRange: NSRangePointer?) -> NSRect {
        if let viewWinFrame = self.window?.convertToScreen(self.frame),
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
    
    func characterIndex(for aPoint: NSPoint) -> Int {
        return 0
    }
    
    override func doCommand(by aSelector: Selector) {
        if (self.responds(to: aSelector)) {
            super.doCommand(by: aSelector);
        } else {
            let commandName = camelCaseToUnderscored(aSelector.description as NSString).replacingOccurrences(of: ":", with: "");
            if (commandName == "noop") {
                NSBeep()
            } else {
                sendRpcAsync(commandName, params: []);
            }
        }
    }

    func cutCopy(_ method: String) {
        let text = sendRpc(method, params: [])
        if let text = text as? String {
            let pasteboard = NSPasteboard.general()
            pasteboard.clearContents()
            pasteboard.writeObjects([text as NSPasteboardWriting])
        }
    }

    func cut(_ sender: AnyObject?) {
        cutCopy("cut")
    }

    func copy(_ sender: AnyObject?) {
        cutCopy("copy")
    }

    func paste(_ sender: AnyObject?) {
        let pasteboard = NSPasteboard.general()
        if let items = pasteboard.pasteboardItems {
            for element in items {
                if let str = element.string(forType: "public.utf8-plain-text") {
                    insertText(str)
                    break
                }
            }
        }
    }

    func undo(_ sender: AnyObject?) {
        sendRpcAsync("undo", params: [])
    }

    func redo(_ sender: AnyObject?) {
        sendRpcAsync("redo", params: [])
    }

    override func mouseDown(with theEvent: NSEvent) {
        removeMarkedText()
        self.inputContext?.discardMarkedText()
        let (line, col) = pointToLineCol(convert(theEvent.locationInWindow, from: nil))
        lastDragLineCol = (line, col)
        let flags = theEvent.modifierFlags.rawValue >> 16
        let clickCount = theEvent.clickCount
        sendRpcAsync("click", params: [line, col, flags, clickCount])
        timer = Timer.scheduledTimer(timeInterval: TimeInterval(1.0/60), target: self, selector: #selector(autoscrollTimer), userInfo: nil, repeats: true)
        timerEvent = theEvent
    }
    
    override func mouseDragged(with theEvent: NSEvent) {
        autoscroll(with: theEvent)
        let (line, col) = pointToLineCol(convert(theEvent.locationInWindow, from: nil))
        if let last = lastDragLineCol, last != (line, col) {
            lastDragLineCol = (line, col)
            let flags = theEvent.modifierFlags.rawValue >> 16
            sendRpcAsync("drag", params: [line, col, flags])
        }
        timerEvent = theEvent
    }

    override func mouseUp(with theEvent: NSEvent) {
        timer?.invalidate()
        timer = nil
        timerEvent = nil
    }

    func autoscrollTimer() {
        if let event = timerEvent {
            mouseDragged(with: event)
        }
    }

    // TODO: more functions should call this, just dividing by linespace doesn't account for descent
    func yToLine(_ y: CGFloat) -> Int {
        return Int(floor(max(y - descent, 0) / linespace))
    }

    func lineIxToBaseline(_ lineIx: Int) -> CGFloat {
        return CGFloat(lineIx + 1) * linespace
    }

    func pointToLineCol(_ loc: NSPoint) -> (Int, Int) {
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

    func updateText(_ text: [String: AnyObject]) {
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
            DispatchQueue.main.async {
                // defer until resize has had a chance to happen
                self.scrollToVisible(scrollRect)
            }
        }
        if self.isFrontmost {
            setInsertionBlink(true)
        }
        needsDisplay = true
    }
    
    /*  Insertion point blinking.
        Only the frontmost ('key') window should have a blinking insertion point.
        A new 'on' cycle starts every time the window is comes to the front, or the text changes, or the ins. point moves.
        Type fast enough and the ins. point stays on.
     */
    
    /// Turns the ins. point visible, and set it flashing. Dose nothing if window is not key.
    func setInsertionBlink(_ on: Bool) {
        // caller must set NeedsDisplay
        cursorFlashOn = on
        blinkTimer?.invalidate()
        if on {
            blinkTimer = Timer.scheduledTimer(timeInterval: TimeInterval(1.0), target: self, selector: #selector(blinkInsertionPoint), userInfo: nil, repeats: true)
        }
        else {
            blinkTimer = nil
        }
    }
    
    // Just performs the actual blinking.
    func blinkInsertionPoint() {
        cursorFlashOn = !self.cursorFlashOn
        needsDisplay = true
    }
    
    // Current color for the ins. point. Implements flashing.
    func cursorColor() -> CGColor {
        if cursorFlashOn {
            return CGColor(gray: 0, alpha: 1) // Black
        }
        else {
            return CGColor(gray: 1, alpha: 1) // should match background.
        }
    }
    
    // Background color for selected text. Only the first responder of the key window should have a non-gray selection.
    func selcolor() -> NSColor {
        if isFrontmost {
            return fgSelcolor
        }
        else {
            return bgSelcolor
        }
    }


    func tryUpdate() {
        var pendingUpdate: [String: AnyObject]?
        updateQueue.sync {
            pendingUpdate = self.pendingUpdate
            self.pendingUpdate = nil
        }
        if let text = pendingUpdate {
            updateText(text)
        }
    }

    func updateSafe(_ text: [String: AnyObject]) {
        updateQueue.sync {
            self.pendingUpdate = text
        }
        DispatchQueue.main.async {
            self.tryUpdate()
        }
    }

    func updateScroll(_ bounds: NSRect) {
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
    func getLine(_ lineNum: Int) -> [AnyObject]? {
        // TODO: maybe core should take care to set height >= 1
        if lineNum < 0 || lineNum >= max(1, self.height) {
            return nil
        }
        if let line = self.lineMap[lineNum] {
            return line
        }
        // speculatively prefetch a bigger chunk from RPC, but don't get anything we already have
        var first = lineNum
        while first > max(0, lineNum - CACHE_FETCH_CHUNK) && lineMap.index(forKey: first - 1) == nil {
            first -= 1
        }
        var last = lineNum + 1
        while last < min(lineNum, height) + CACHE_FETCH_CHUNK && lineMap.index(forKey: last + 1) == nil {
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
                    lineMap[lineNum] = ["" as AnyObject]  // TODO: maybe core should always supply
                }
            }
        }
        return lineMap[lineNum]
    }
    
    func fetchLineRange(_ first: Int, _ last: Int) -> [[AnyObject]]? {
        let start = Date()
        if let result = sendRpc("render_lines", params: ["first_line": first, "last_line": last]) as? [[AnyObject]] {
            let interval = Date().timeIntervalSince(start)
            Swift.print(String(format: "RPC latency = %3.2fms", interval as Double * 1e3))
            return result
        } else {
            Swift.print("rpc error")
            return nil
        }
    }
    
    var isEmpty: Bool {
        if height == 0 { return true }
        if height > 1 { return false }
        if let line = getLine(0) {
            return line[0] as? String == ""
        } else {
            return true
        }
    }
    
    func updateIsFrontmost(_ frontmost : Bool) {
        isFrontmost = frontmost
        setInsertionBlink(isFrontmost)
        needsDisplay = true
    }
    
    // MARK: - Debug Methods

    @IBAction func debugRewrap(_ sender: AnyObject) {
        sendRpcAsync("debug_rewrap", params: [])
    }

    @IBAction func debugTestFGSpans(_ sender: AnyObject) {
        sendRpcAsync("debug_test_fg_spans", params: [])
    }

    @IBAction func debugRunPlugin(_ sender: AnyObject) {
        sendRpcAsync("debug_run_plugin", params: [])
    }
}
