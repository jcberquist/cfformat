<cffunction name="f">
<cfif a><cfset var y = 1>
<cfelseif b><cfset y = 2>
<cfelse><cfset y = 3></cfif>
<cftry><cfset var t = 1>
<cfcatch><cfset t = 2></cfcatch>
<cffinally><cfset t = 3></cffinally></cftry>
<cfset y = 4>
</cffunction>
