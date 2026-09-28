<div>
<div>
<cfquery name="q" datasource="#application.dsn#">
SELECT id, name
FROM   people
WHERE  id = <cfqueryparam value="#arguments.personIdentifierFromTheRequest#" cfsqltype="cf_sql_integer" null="#not len(arguments.personIdentifierFromTheRequest)#">
  AND  kind = <cfqueryparam value="#kind#" cfsqltype="cf_sql_varchar">
<cfif len(arguments.status)>
  AND  status = <cfqueryparam value="#arguments.statusCodeForThePersonRecord#" cfsqltype="cf_sql_varchar" maxlength="50">
</cfif>
</cfquery>
</div>
</div>
