<cfif x>
<cfquery>/* ' */ select 'a
  b' /* ' */ as x</cfquery>
<cfquery>select $$a
  b$$ as x</cfquery>
<cfquery>select 'it''s' as a,
  'b' as b</cfquery>
<cfquery>select 'a\'
  b' as x</cfquery>
<cfquery>select [it's] as a,
  1 as b</cfquery>
<cfquery>select /* a /* b */ 'c
  d' */ 1</cfquery>
<cfquery>
select 1 -- it's
from t
</cfquery>
</cfif>
