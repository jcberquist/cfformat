// script
if (
    !skipped
    && structKeyExists(input, "key")
    // The first line of a comment run,
    // (the second line
    // and the third).
    && compareKeys(input.key, expectedKey)
) {
    matched = true;
}
var x = a
    // why a
    // and more
    ? b
    // why b
    /* block */
    // last
    : c;
y = // one
// two
1;
return first /* b1 */ // l1
    // l2
    /* b2 */
    + second;
t = a ? b : // after the colon
    c;
u = a ? // after the question mark
    b : c;
v = - // after a prefix operator
    a;
w = a. // inside a member access
    b;
if (a) x = 1 // before the semicolon
;
items.each((item) => total // before the operator
+= item);
local. // inside the target
f = arguments.cb(a, b)?.c;
x // before the operator
= 1;
s = // before a value that breaks
{ a: 1, f: function() {} };
