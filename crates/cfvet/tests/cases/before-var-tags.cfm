<cffunction name="a">
    <cfset x = 1>
    <cfset var x = 2>
    <cfloop index="i" from="1" to="3">
        <cfset x = x + i>
    </cfloop>
    <cfset var i = 0>
    <cfreturn x>
</cffunction>
