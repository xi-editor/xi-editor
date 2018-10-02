---
layout: page
title: Rope science, part 4 - parenthesis matching
site_nav_category_order: 211
is_site_nav_category2: true
site_nav_category: docs
---

(originally written 20 Apr 2016)

One of features I find most enjoyable and useful in a modern editor is highlighting the matching parenthesis or bracket. Parenthesis matching is also used for code folding (which I barely use, but I guess some people like), and is one of the primitives useful in indentation, essential in a code editor. In Unicode processing, parenthesis matching is (now) part of BiDi, and there’s some talk of standardizing on heuristics for font selection that keep parentheses matches. So it’s an important primitive.

Let’s start by looking at a highly simplified version of the problem. Your language has only three characters in it: “(” “other”, and “)”. These correspond to a nesting balance of +1, 0, and -1, respectively. Obviously to find the nesting balance of a string you can keep a running sum from beginning to end, but we’re trying to avoid making things more sequential than they have to be, and indeed, the simple and familiar integer sum (this time signed integer rather than unsigned) makes this into a monoid which can then be computed in parallel or incrementally.

However, that monoid is not quite powerful enough to find the matching parenthesis. Let’s say the cursor is on an open paren and you want to find the close. Then, the problem can be defined as finding the shortest nonempty substring starting at your cursor that has a balance of 0. Again, in a sequential scan this is trivial, but we’re trying not to do that.

Fortunately, it’s still very easy to turn this problem into a monoid. You keep track of both total and minimum nesting level. Formally, the monoid sum of (t1, m1) and (t2, m2) is (t1 + t2, min(m1, t1 + m2)). Note that this monoid is non-commutative - “()” has a value of (0, 0) while “)(” is (0, -1). That non-commutativity is key. It captures the idea of state, the idea that the ordering matters. Guy Steele talks about this in his excellent [talk on catamorphisms](https://groups.csail.mit.edu/mac/users/gjs/6.945/readings/MITApril2009Steele.pdf), and points out that MapReduce itself is limited to commutative operations. When I talk about “MapReduce for strings” I’m very much including non-commutative monoids as well.

The projection of this monoid onto just the minimum field is monotonic. As you keep appending more string, the value can go down, never up. So, if your string is stored in a rope data structure and you store (and update) this monoid in every node of the tree, you can search in O(log n) for a given transition, in this case 1 -> 0. The [finger tree paper](http://www.staff.city.ac.uk/~ross/papers/FingerTree.html) goes into more detail of monotonic monoids and how to scan for them efficiently (indexing a random position is a special case, where the monoid is integer sum, but the idea generalizes quite nicely).

Dan Piponi goes into more of the mathematical detail on this parenthesis matching problem in his blog post [Beyond Regular Expressions](http://blog.sigfpe.com/2009/01/beyond-regular-expressions-more.html). But I’m going to turn a little to how to adapt this to real programming languages, rather than such a simplified fragment.

The first problem is comments (quoted string literals are similar). A parenthesis appearing in a comment doesn’t count to the nesting balance, but it’s hard to tell from looking at a substring whether it’s in a comment or not. Let’s augment our language with two more characters, “begin comment” (or “#”) and “end comment” (or “\n”). A comment is defined as everything from the begin to the first end after, and within a comment parentheses have a nesting balance of 0, just like “other”.

Again, making this into a monoid is pretty easy. You store two copies of the (t, m) pair - one for the simple case, and one for the case where the beginning of the string is in a comment. You also keep two bits to keep track of whether the string ends or begins a comment. In principle, you have to do the computation twice for both cases, whether the first line is a comment or not, but in practice it doesn’t make the computation any more expensive: you compute (t, m) for the first line and for the rest of the string, and just store both the first value and the monoid sum.

The same tricks extend without too much difficulty to a real language like C++; there are more cases but the lexical language for comments and quoted strings tends not to be too complicated. There’s also the complication of multiple types of parentheses: (), [], {}, but my thinking on this is to store an integer count for all of them (unfortunately <> is probably a lost cause). To deal with mismatched parens (which are usually going to be an error of some kind), I think what you want to do is retrieve the local neighborhood of each of your matches, then you can easily tell just from the character whether it’s matched or not. I hypothesize that having the comment and string context, the local neighborhood of paren matches, and total depth, is adequate to produce high quality indenting in most languages.

I’ll probably implement some form of this, most likely as an API that plugins can query. But it's certainly a demonstration of how even simple monoids can capture more complex and interesting structure within strings.﻿

### In comments

Jonathan Tomer pointed out that real parsing is much more interesting than just paren
matching.

Peter Ludemann brought up [Dyck language](https://en.wikipedia.org/wiki/Dyck_language)
