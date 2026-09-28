<cfcomponent>
<cffunction name="f">
    <cfargument name="a">
    <cfset var x = 1>
    <cfset y = 2>
    <cfset local.z = 3>
    <cfquery name="q" datasource="d">select 1</cfquery>
    <cfloop index="i" from="1" to="3"></cfloop>
    <cfloop item="it" array="#arr#"></cfloop>
    <cfloop query="q"></cfloop>
    <cfsavecontent variable="sc"></cfsavecontent>
    <cfparam name="pp" default="1">
    <cfhttp url="u" result="r"></cfhttp>
    <cfif true><cfset w = 1></cfif>
</cffunction>
</cfcomponent>
