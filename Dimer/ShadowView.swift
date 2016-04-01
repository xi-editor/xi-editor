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

    override func drawRect(dirtyRect: NSRect) {
        let context = NSGraphicsContext.currentContext()!.CGContext
        let colors = [CGColorCreateGenericRGB(0, 0, 0, 0.4), CGColorCreateGenericRGB(0, 0, 0, 0.0)]
        let colorLocations: [CGFloat] = [0, 1]
        let gradient = CGGradientCreateWithColors(CGColorSpaceCreateDeviceRGB(), colors, colorLocations)
        CGContextDrawLinearGradient(context, gradient, NSPoint(x: 0, y: 0), NSPoint(x: 0, y: 3), [])
    }

    override var flipped: Bool {
        return true;
    }
    
}
