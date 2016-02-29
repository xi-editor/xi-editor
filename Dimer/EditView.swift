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

    var text: String?

    override func drawRect(dirtyRect: NSRect) {
        super.drawRect(dirtyRect)
        let path = NSBezierPath(ovalInRect: dirtyRect)
        NSColor.greenColor().setFill()
        path.fill()
        let font = NSFont(name: "Helvetica", size: 14.0)
        let baselineAdjust = 1.0
        let attrsDictionary = [NSFontAttributeName: font!, NSBaselineOffsetAttributeName: baselineAdjust] as [String : AnyObject]
        let str:NSString = text ?? "(none)"
        str.drawInRect(dirtyRect, withAttributes: attrsDictionary)
        NSLog("drawRect called %f %f %f %f", dirtyRect.origin.x, dirtyRect.origin.y, dirtyRect.width, dirtyRect.height)
        // Drawing code here.
    }
    
    override var acceptsFirstResponder: Bool {
        return true;
    }
    
    override func keyDown(theEvent: NSEvent) {
        text = text ?? ""
        let characters = theEvent.characters ?? "(none)"
        let keycode = theEvent.characters?.unicodeScalars.first?.value ?? 0;
        if keycode == 0x7f {
            if (!text!.isEmpty) {
                text = text!.substringToIndex(text!.endIndex.predecessor())
                needsDisplay = true
            }
        } else if (keycode & 0xf700) != 0xf700 {
            text!.appendContentsOf(characters)
            needsDisplay = true
        }
        NSLog("keyDown: %d %x %@", theEvent.keyCode, theEvent.modifierFlags.rawValue, characters)
        for c in characters.unicodeScalars {
            NSLog("char %x", c.value)
        }
    }
    
}
