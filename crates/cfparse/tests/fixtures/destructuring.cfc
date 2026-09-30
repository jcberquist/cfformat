//
[a, b] = x;
[a, , c] = x;
[a, ...r] = x;
[a, [b, c], {d}] = x;
[a, b = 9] = x;
[a, b] = [b, a];
({a, b: c, d = 1, e: f = 2, ...r} = x);
({p: {q, r: [s, t]}} = x);
({a, b,} = x);
[a, b,] = x;
var [a, , c] = x;
var {p, q: {r, s = 1}, ...t} = st;
final [a, b] = x;
static {a, b} = x;
for ([k, v] in pairs) {}
for ({a, b} in structs) {}
for (var [k, v] in pairs) {}
for (var {a, b} in structs) {}
({
    a, // first
    b: c /* renamed */,
    d = 1
} = x);
function f(x, {a, b = 9, p: {q}, ...r}, {c}) {}
function g({a, b: [c, d]} = {}, y) {}
h = function({a, b}) {
    return a + b;
};
k = ({a, b}) => a + b;
m = ({a, b = 2}, c) => { return a + b + c; };
