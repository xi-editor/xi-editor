---
layout: page
title: Rope science, part 3 - Grapheme cluster boundaries
site_nav_category_order: 210
is_site_nav_category2: true
site_nav_category: docs
---

(originally written 16 Apr 2016)

One of the trickier problems in text processing is computing grapheme cluster boundaries. A nontrivial portion of the N release cycle has been spent fixing them and applying them consistently in the Android text stack.

Another reason why grapheme cluster boundaries are interesting is that Apple has basically deemed them the new “character” in Swift. I think that’s misguided, for reasons that should become clear soon.

To a first approximation, a grapheme cluster is what you want to step over when you press an arrow key. In ASCII, every character is also its own grapheme, but in many other scripts you can combine multiple codepoints together. The most familiar example is adding combining marks to a base character, like placing accents on letters. For example, if you had U+0061 (a) and U+0301 (acute accent), you definitely wouldn’t want to be able to step the cursor between them. Unicode makes things even more fun by giving this combination its own codepoint (U+00E1) and recommending that text processing systems treat both forms the same.

For the most part, boundaries between grapheme clusters are defined in UAX #29. Basically, for every pair of codepoints, you look up the Grapheme_Cluster_Break property in your favorite Unicode database, then apply a table of rules to see whether that pair entails a break between the two codepoints. So, for example, between an alphabetic base character and a combining mark, no break. Between two alphabetic characters, a break.

Would that life were so simple. There are a bunch of exceptions to these rules we’ve come up with, for improving the behavior in a number of complex scripts, but those pale in comparison to the complete mess that emoji brings.

Including recent draft standards, there are no fewer than six different ways to form compound emoji from multiple Unicode codepoints: enclosing keycap, variation selector, skin tone modifiers, ZWJ sequences, tags for customized emoji, and flags. All of these have subtly different Unicode properties, many of which are quite broken and in the process of being fixed. The general principle is that you want such a compound emoji to behave similarly to a single-codepoint emoji, and defining grapheme cluster boundaries are one of the main ways we do that.

Of the six types of compound emoji, flags have a special place in my heart. A flag is defined by two characters in a “Regional Identifier Symbol” space homomorphic to A-Z. For a long sequence of RIS codepoints, you expect to them to pair up; the boundaries are at the even-numbered offsets within the sequence. Since you can no longer tell from looking at a single pair whether there’s a boundary between them, [UAX #29](http://www.unicode.org/reports/tr29/tr29-27.html) [note added later: link is to the Unicode 8 version, which was current at the time this was written] gives up and says the whole thing is a single grapheme cluster. So, if you believe the spec, then pressing the arrow key at the beginning of a sequence of flags jumps over them all. (Fun fact, this was actually the source of a hang bug in the SMS app, because it assumed that the result of the Java Character iterator would fit inside one SMS fragment) [edit added later: the [fix for this bug](https://android.googlesource.com/platform/frameworks/opt/telephony/+/bee1df8) is now released in AOSP; also, the grapheme cluster rules have changed in the [Unicode 9 version of UAX #29](http://www.unicode.org/reports/tr29/tr29-29.html), so Android and Unicode are now in sync]

We’ve decided, as part of our emoji polish effort, that such behavior is not good enough, so our grapheme break detector scans backward through the text and counts by two. However, this is potentially the source of some O(n^2) behavior, so we limit it to some reasonable number; after that, they all glue into a single cluster.

If you really wanted to solve the full problem, the MapReduce framework would do the job nicely; your summary info is two bits, one indicating whether the suffix of RIS codepoints is even or odd in length, and another indicating whether there's a non-RIS codepoint, with combination rules that follow pretty directly from this definition. That way, no matter how pathological the test document becomes, you'd get the right answer in microseconds.

I’m not sure yet whether I’ll implement this in xi – it’s pretty tempting just to look at a finite window, as that solves 99.99% of the problem. But it would make a pretty fun demonstration, and it’s quite easy; I’ll almost certainly test for the presence of RIS codepoints as part of the difficulty analysis, and these two extra bits in the monoid won’t hurt at all.

Back to Swift. If you’re doing serious text processing and want to get the grapheme analysis right, you can’t rely on the language, you have to override the behavior using your own code. Further, they now have a tough decision to make. Either they stick to UAX #29 and have poor behavior, or they implement look-behind, potentially introducing O(n^2) behavior and making programs vulnerable to text-based denial of service. Further, they’d make the result of string.characters.count depend on the language version, which would cause even more problems if you’re relying on indexes within a string to persist across storage and RPC boundaries. The rules for grapheme boundaries are going to have to change in any case as Unicode revs. I think having a method in your string class for breaking is fine, but having it as a core property of your string type (in many ways, raised up as the preferred view) is a mistake.

In any case, it was fun to think about this example of a thorny problem which ends up fitting nicely into the MapReduce framework after all.﻿
