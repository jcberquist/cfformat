//
function make() {
    var z = new component {
        function test(){ throw "inline"; }
    }
    var cfc = new component accessors="true" output=false {
        property name="a";
        function g() { return 1; }
    };
    var prop = a.b.c ?: new component {};
    return new Component javaSettings='{"maven":["a:b:1"]}'{
        import java.util.Date;
    };
}
