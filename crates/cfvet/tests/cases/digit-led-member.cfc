component {
    function f() {
        // A member name may start with a digit: these write `variables.0bar`,
        // `local.0baz` and a member of `cfg`, never a variable `bar` or `baz`.
        variables.0bar = 1;
        local.0baz = 2;
        var cfg = {};
        cfg.0qux = local.0baz;
        other.0name = 3;
    }
}
