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

class ShadowView: NSView {
    var topShadow = false
    var leadingShadow = false
    var trailingShadow = false
    
    var mouseUpHandler: ((NSEvent) -> Void)?
    var mouseDraggedHandler: ((NSEvent) -> Void)?

    override func drawRect(dirtyRect: NSRect) {
        if topShadow || leadingShadow || trailingShadow {
            let context = NSGraphicsContext.currentContext()!.CGContext
            let colors = [CGColorCreateGenericRGB(0, 0, 0, 0.4), CGColorCreateGenericRGB(0, 0, 0, 0.0)]
            let colorLocations: [CGFloat] = [0, 1]
            let gradient = CGGradientCreateWithColors(CGColorSpaceCreateDeviceRGB(), colors, colorLocations)!
            if topShadow {
                CGContextDrawLinearGradient(context, gradient, NSPoint(x: 0, y: 0), NSPoint(x: 0, y: 3), [])
            }
            if leadingShadow {
                CGContextDrawLinearGradient(context, gradient, NSPoint(x: 0, y: 0), NSPoint(x: 3, y: 0), [])
            }
            if trailingShadow {
                let x = bounds.size.width
                CGContextDrawLinearGradient(context, gradient, NSPoint(x: x - 1, y: 0), NSPoint(x: x - 4, y: 0), [])
            }
        }
    }

    override var flipped: Bool {
        return true;
    }

    func updateScroll(contentBounds: NSRect, _ docBounds: NSRect) {
        let newTop = contentBounds.origin.y != 0
        let newLead = contentBounds.origin.x != 0
        let newTrail = contentBounds.origin.x + contentBounds.width != docBounds.origin.x + docBounds.width
        if newTop != topShadow || newLead != leadingShadow || newTrail != trailingShadow {
            needsDisplay = true
            topShadow = newTop
            leadingShadow = newLead
            trailingShadow = newTrail
        }
    }

    override func mouseDragged(theEvent: NSEvent) {
        mouseDraggedHandler?(theEvent)
    }

    override func mouseUp(theEvent: NSEvent) {
        mouseUpHandler?(theEvent)
    }

}
