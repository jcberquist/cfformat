component {

    function statements(x) {
        var [p, q = 1, [r]] = x;
        var {s, t: u = 2, ...v} = x;
        [a, b = 1, ...c] = x;
        ({d, e: f = 2, ...g} = x);
        for (var [k, l] in x) {}
        for ([m, n] in x) {}
        var [w = (leak = 1)] = x;
        [p, q] = [q, p];
        return p + q + r + s + u + v + k + l + w;
    }

    function parameters({a, b = 1, p: {c}}, d, {e} = {}) {
        a = 2;
        d = 3;
        return a + b + c + d + e;
    }

    function closures() {
        var h = ({i}) => i;
        var j = function({k = 2}) {
            return k;
        };
        return h({i: 1}) + j({});
    }

}
