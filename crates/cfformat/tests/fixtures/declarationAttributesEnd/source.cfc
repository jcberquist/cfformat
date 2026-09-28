// script
abstract component { function f() }
interface {
    function f()
    public void function g()
    function h()
        output=false
        abstract
    function i() hint="x"}
component {
    property name="a" type="string" // before the semicolon
    ;
    param name="b" default="1" // before the semicolon
    ;
    function g // between the name and the parameters
    () {
        lock name="x" timeout="5" // before the block
        {
            s = { f: function k // between the name and the parameters
                () {} };
        }
    }
    function h /* between */ () {}
}
foo(a, function // before the parameters
() { return 1; });
foo(a, function() // before the body
{ return 1; });
