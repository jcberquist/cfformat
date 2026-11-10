<cfscript>
    foo.0bar = 123;
    foo.999bar = 456;
</cfscript>
<cfoutput>#foo.0bar#</cfoutput>
<cfoutput>#foo.999bar#</cfoutput>
<cfset foo.0bar = 1>
<cfif foo.0bar eq 1>x</cfif>
<cfoutput>#foo.0bar.baz# #foo.1# #foo?.0bar#</cfoutput>
<cfloop collection="#foo.0bar#" item="k"></cfloop>
<cfscript>
    x = foo.0bar;
    foo.0bar++;
    foo.0bar += 1;
    x = foo.0bar[1];
    x = foo.0bar ?: 1;
    foo.0bar();
    foo.0bar = function() { return 1; };
    variables.0bar = 1;
    x = a.1b.2c.3;
    x = arr[1].0bar;
    x = mk().0bar.baz;
    x = Comp::0bar;
    x = foo?.0bar;
    x = foo?.0bar?.1baz;
    x = foo.1e5;
    x = foo.0x1F;
    x = foo.1e+5;
    x = foo.0-bar;
    x = c ?.5 : 2;
    x = foo?.5;
    x = foo?.1.bar;
    a = {0bar: 1, 0_bar = 2, 0x1F: 3, 1E5x: 4};
    b = [0bar: 1, 1: 2];
    d = f(0bar = 1, 0baz: 2);
    result = service.0first().1second(argumentOne).2third(argumentTwo, argumentThree).4fourth(argumentFour).5fifth.6sixth().toList();
</cfscript>
