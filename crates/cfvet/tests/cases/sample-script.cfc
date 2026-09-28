component {
    function init(required string dsn) {
        var a = 1;
        local.b = 2;
        c = 3;
        d.e = 4;
        f[1] = 5;
        g++;
        h += 1;
        param name="p" default=1;
        param string q = 1;
        for (i = 1; i <= 3; i++) {}
        for (var j = 1; j <= 3; j++) {}
        for (k in s) {}
        for (var m in s) {}
        try {} catch (any err) {}
        query name="qry" datasource="x" { echo("select 1"); }
        http url="x" result="res";
        savecontent variable="sc" {}
        cfhttp(url="x", result="res2");
        arrayEach([1], function(x) { y = x; var z = x; });
        var fn = (x) => x;
        arguments.dsn = 1;
        variables.v = 1;
        this.t = 1;
        setVariable("dyn", 1);
        return c;
    }
}
