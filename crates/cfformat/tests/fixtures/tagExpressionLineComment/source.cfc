<cfif a>
<cfset y = 1 // runs past > to the end
>
<cfset z = 2 // self-closed
/>
<cfset w = a + b // at the end of a binary
>
<cfif x // a condition
>
q
</cfif>
<cfquery name="q">
    SELECT a FROM t
    <cfif x // in an island
    >WHERE b = 1</cfif>
</cfquery>
</cfif>
<cfset s // before the value
= { a: 1, b: [1, 2] }>
<cfreturn new // before the name
Foo(a, b)>
<cfset x = foo(a ? b : // inside the arguments
c, [1, 2])>
