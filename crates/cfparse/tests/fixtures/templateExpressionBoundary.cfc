<cfoutput>
<p>#"#a#"# and #DateFormat(#Now()#, "yyyy")# and ##</p>
<a href="page.cfm?x=#x#&amp;y=##" title="#a /* # */ + b#">#items[1]#</a>
</cfoutput>
<cfloop from="#a#" to="#b + 1#" index="i"></cfloop>
