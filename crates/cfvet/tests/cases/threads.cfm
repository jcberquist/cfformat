<cffunction name="f">
<cfscript>
var a = 1;
b = 2;
thread name="t" { c = 3; }
</cfscript>
<cfthread name="u"><cfset d = 4></cfthread>
</cffunction>
