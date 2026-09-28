<cffunction name="f">
<cfif a><cfset var x = 1>
<cfelseif b><cfset var x = 2>
<cfelse><cfset var x = 3></cfif>
<cfset x = 4>
<cfif a><cfset var y = 1>
<cfelseif b><cfset var y = 2></cfif>
<cfset y = 3>
<cfswitch expression="#a#">
<cfcase value="1"><cfset var w = 1></cfcase>
<cfdefaultcase><cfset var w = 2></cfdefaultcase>
</cfswitch>
<cfset w = 3>
<cfswitch expression="#a#">
<cfcase value="1"><cfset var v = 1></cfcase>
<cfcase value="2"><cfset var v = 2></cfcase>
</cfswitch>
<cfset v = 3>
</cffunction>
