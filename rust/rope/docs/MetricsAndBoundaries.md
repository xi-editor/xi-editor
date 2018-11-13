# Metrics and Boundaries
The xi-rope code tries to capture general underlying mathematical concepts without being overly abstract. This can be confusing (especially to newcomers) and there are definitely ways in which the current code doesn't live up to this ideal.

## Monoid homomorphisms
A rope is a data structure for representing strings, so that many operations (especially editing) are efficient even as the string grows. Xi-rope implements strings, but also generalizes to other structures.

The mathematical generalization of a string is a monoid. The operation is string concatenation, and the identity element is the empty string.

A string has a length. The empty string has zero length, and the length of the concatenation of two strings is the sum of their lengths. The mathematical generalization of this idea is a monoid homomorphism, which basically says that the function preserves the monoid structure.

The concept of monoid homomorphism doesn't cover everything we care about regarding strings. In particular, it's missing any way to compute substrings, and we care about that deeply.

## Metrics
A metric is a monoid homomorphism ranging over nonnegative integers, and also satisfying a form of the triangle inequality. If we write the monoid operation as ⊕, then this is simply a ⊕ b ≤ a + b. String length is an important metric, but there are other interesting examples.

A metric is additive if its operation is addition. Any additive metric trivially satisfies the triangle inequality.

A good example of a nonadditive metric is the "nonascii" metric, defined as being 0 for a string that is entirely ASCII, and 1 if it contains any non-ASCII character. Here, the monoid operator could be written a ⊕ b = min(a + b, 1). (Note: maybe this is confusing, and metrics should be reserved for nonnegative integers with addition, and some other concept. That said, as we define boundaries in terms of metric it will probably be useful.)

An atomic metric is one that is nonzero for any element of the original monoid other than the identity element. A good way to explore atomicity is to look at representations of Unicode strings. (Note: given how often I say "additive atomic" and that I can't think of a good use case for a non-additive atomic metric, it might be worth not separating these concepts out so much)

If the monoid can only represent valid Unicode strings, i.e. sequences of valid code points, then the obvious metric is to count the number of code points. However, it's usually more efficient to store the string as a sequence of code units rather than code points, and in that case it might be more efficient to count code units (as this can be used to index directly into the representation). Counting code points, UTF-8 code units, and UTF-16 code units are all acceptable atomic metrics.

Another choice, however, is to allow the monoid to contain arbitrary bytes, not just valid Unicode strings. This might be useful to allow leaves to contain fixed-size blocks, which might require splitting code points. The metric that counts bytes in the range [0..0x7F] and [0xC0..0xFF] successfully counts code points, but is not atomic. In particular, a single 0x80 byte is not the empty string, but has zero measure in this metric.

Another important non-atomic metric is the count of newline characters.

(Note: probably clearer to just motivate the newline metric here and go into UTF-8 below)

## Split
A defining characteristic of strings is that they can be split, which is something of an inverse of the monoid operator. To generalize the concept, we'll need an atomic metric m() that is also additive (the monoid operator is addition). The split function can be defined as:

split(a ⊕ b, m(a)) = (a, b)

(Conjecture: this split function is uniquely defined for any monoid which is cancellative and any additive atomic metric.)

From split we easily get substring, and also all the interesting editing operations.

## Base units
The Tree struct in xi-rope models a general monoid homomorphism, plus an additive atomic metric. This is known as the base metric, and measures base units. The base units often correspond to bytes in the string representation, but needn't. In any case, the measure of a Tree in base units is referred to as its len.

Many operations, including substring, refer to a location within the structure in base units. The substring operation is implemented as the push_maybe_split method on Leaf, and the indices given as arguments are base units.

Each valid location in a string, other than 0, has a previous location, and each other than its len has a next location, all measured in base units. Taking next then previous lands you in the same location, and vice versa. Take as a concrete example "\u{00A1}!" represented as the UTF-8 byte sequence [0xC2, 0x81, 0x21]. The next location after 0 is 2, and the next location after 2 is 3. The only valid locations in this string are 0, 2, and 3.

## Boundaries
In text editing, we are very often concerned with finding boundaries in the text, for example those induced by newline characters. Such boundaries can be defined in terms of a metric, and here we'll explore in greater detail.

For full generality, we need both the concept of leading and trailing boundaries.

A location x is a trailing boundary with respect to some metric when the measure of the substring from prev(x) to x is nonzero. In addition, the location 0 is always considered a trailing boundary. Conversely, x is a leading boundary when the substring from x to next(x) has nonzero measure, and len is always a leading boundary.

Note that, for an atomic metric, all valid locations are boundaries, and there is no distinction between leading and trailing boundaries.

Consider the metric that counts newlines. Then, in the string "abc\ndef", the trailing boundaries are 0 and 4, corresponding to the starts of the first and second lines. There is no trailing boundary at the end of this string, though there would be for "abc\ndef\n".

One application of leading boundaries is UTF-8 string representation when splitting code points is allowed. In that case, the metric counts bytes in the range [0..0x7F] and [0xC0..0xFF] (i.e. the bytes that can start a code point). For strings that are valid UTF-8, the boundaries between code points are the leading boundaries with respect to this metric.

Should the choice between leading and trailing boundary be considered an inherent property of the metric (so would be trailing for newlines and leading for UTF-8), or in the way the metric is used? The current code is quite confused on this, and since xi uses currently uses trailing boundaries almost exclusively, there are gaps in the implementation of leading boundaries.

Here is an example where both leading and trailing boundaries are useful from the same metric. Consider the nonascii metric above, and imagine code which has a "fast path" for ascii and a "slow path" for general unicode. Such an algorithm might alternate between finding the next leading edge, and processing that interval using the fast path, and then finding the next trailing edge, processing using the slow path.

(Edit on further thinking: the definitions above will support fast scanning for large contiguous zero measure (fast path) ranges, but not for large contiguous nonzero measure ranges. I think that's ok, but needs to be clarified. If you're scanning forward, then only the leading edge is useful.)

There current code needs to be fixed to support this. Probably the more systematic would be to explicitly request leading or trailing edge (likely by providing an additional explicit argument to the is_boundary, next, and prev methods of Cursor). Another approach is to bind the preference for leading or trailing boundary into the metric (this is closest to the way the code is currently structured), then require two separate instances of Metric to support the fast-path use case above.

