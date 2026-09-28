<cftransaction>
<cfquery name="q">UPDATE t SET a = 1</cfquery>
<cfif ok>
<cftransaction action="commit">
<cfelse>
<cftransaction action="rollback" />
</cfif>
</cftransaction>
<cfthread action="run" name="t">
<cfthread action="sleep" duration="100">
<cfset x = 1>
</cfthread>
<cfthread action="join" name="t">
<cftransaction><cftransaction action="commit"></cftransaction>
