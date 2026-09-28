// script
component {
    function f(a) hint="x" type="java" output=false {
        return a.length();
    }
    public String function g(String s) type="java" {
        if (s == null) { return ""; }
        return s.trim();
    }
    function h() type=java;
}
