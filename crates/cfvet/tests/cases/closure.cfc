component {
    function sum(required array xs) {
        var total = 0;
        arrayEach(xs, function(x) {
            // `total` is the enclosing function's local; `seen` is not.
            total += x;
            seen = true;
            var inner = x;
        });
        inner = 1;
        var twice = (x) => {
            doubled = x * 2;
            return doubled;
        };
        return total;
    }
}
