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

// A data structure holding a cache of lines, with methods for updating based
// on deltas from the core.

import Foundation

struct Line {
    var text: String
    var cursor: [Int]
    var styles: [Int]

    init(fromJson json: [String: AnyObject]) {
        if let text = json["text"] as? String {
            self.text = text
        } else {
            self.text = ""  // this should probably be an exception
        }
        if let cursor = json["cursor"] as? [Int] {
            self.cursor = cursor
        } else {
            self.cursor = []
        }
        if let styles = json["styles"] as? [Int] {
            self.styles = styles
        } else {
            self.styles = []
        }
    }

    init?(updateFromJson line: Line?, json: [String: AnyObject]) {
        if let line = line {
            self.text = line.text
            if let cursor = json["cursor"] as? [Int] {
                self.cursor = cursor
            } else {
                self.cursor = line.cursor
            }
            if let styles = json["styles"] as? [Int] {
                self.styles = styles
            } else {
                self.styles = line.styles
            }
        } else {
            return nil
        }
    }
}

// Note: all public methods of this class are designed to be thread-safe
class LineCache {
    private let queue = DispatchQueue(label: "com.levien.xi.LineCache")
    var nInvalidBefore = 0;
    var lines: [Line?] = []
    var nInvalidAfter = 0;

    var height: Int {
        get {
            return queue.sync { heightLocked }
        }
    }

    private var heightLocked: Int {
        get {
            return nInvalidBefore + lines.count + nInvalidAfter
        }
    }

    func get(_ ix: Int) -> Line? {
        return queue.sync { getLocked(ix) }
    }

    private func getLocked(_ ix: Int) -> Line? {
        if ix < nInvalidBefore { return nil }
        let ix = ix - nInvalidBefore
        if ix < lines.count {
            return lines[ix]
        }
        return nil
    }

    func applyUpdate(update: [String: Any]) {
        queue.sync { applyUpdateLocked(update: update) }
    }

    private func applyUpdateLocked(update: [String: Any]) {
        guard let ops = update["ops"] else { return }
        var newInvalidBefore = 0
        var newLines: [Line?] = []
        var newInvalidAfter = 0
        var oldIx = 0;
        for op in ops as! [[String: AnyObject]] {
            guard let op_type = op["op"] as? String else { return }
            guard let n = op["n"] as? Int else { return }
            switch op_type {
            case "invalidate":
                if newLines.count == 0 {
                    newInvalidBefore += n
                } else {
                    newInvalidAfter += n
                }
            case "ins":
                for _ in 0..<newInvalidAfter {
                    newLines.append(nil)
                }
                newInvalidAfter = 0
                guard let json_lines = op["lines"] as? [[String: AnyObject]] else { return }
                for json_line in json_lines {
                    newLines.append(Line(fromJson: json_line))
                }
            case "copy", "update":
                var nRemaining = n
                if oldIx < nInvalidBefore {
                    let nInvalid = min(n, nInvalidBefore - oldIx)
                    if newLines.count == 0 {
                        newInvalidBefore += nInvalid
                    } else {
                        newInvalidAfter += nInvalid
                    }
                    oldIx += nInvalid
                    nRemaining -= nInvalid
                }
                if nRemaining > 0 && oldIx < nInvalidBefore + lines.count {
                    let nCopy = min(nRemaining, nInvalidBefore + lines.count - oldIx)
                    let startIx = oldIx - nInvalidBefore
                    if op_type == "copy" {
                        newLines.append(contentsOf: lines[startIx ..< startIx + nCopy])
                    } else {
                        guard let json_lines = op["lines"] as? [[String: AnyObject]] else { return }
                        var jsonIx = n - nRemaining
                        for ix in startIx ..< startIx + nCopy {
                            newLines.append(Line(updateFromJson: lines[ix], json: json_lines[jsonIx]))
                            jsonIx += 1
                        }
                    }
                    oldIx += nCopy
                    nRemaining -= nCopy
                }
                if newLines.count == 0 {
                    newInvalidBefore += nRemaining
                } else {
                    newInvalidAfter += nRemaining
                }
                oldIx += nRemaining
            case "skip":
                oldIx += n
            default:
                print("unknown op type \(op_type)")
            }
        }
        nInvalidBefore = newInvalidBefore
        lines = newLines
        nInvalidAfter = newInvalidAfter
    }

    // Return ranges of invalid lines within the given range
    func computeMissing(_ first: Int, _ last: Int) -> [(Int, Int)] {
        return queue.sync { computeMissingLocked(first, last) }
    }

    private func computeMissingLocked(_ first: Int, _ last: Int) -> [(Int, Int)] {
        var result: [(Int, Int)] = []
        let last = min(last, heightLocked)  // lines past the end aren't considered missing
        for ix in first..<last {
            // could optimize a bit here, but unlikely to be important
            if ix < nInvalidBefore || ix >= nInvalidBefore + lines.count || lines[ix - nInvalidBefore] == nil {
                if result.count == 0 || result[result.count - 1].1 != ix {
                    result.append((ix, ix + 1))
                } else {
                    result[result.count - 1].1 = ix + 1
                }
            }
        }
        return result
    }
}
