<cfoutput>
<script type=
"#kind#">hello</script>
<script type
="#kind#">hello</script>
<script #attrs#>hello</script>
<script defer #attrs#>hello</script>
<script src=#url#>hello</script>
<script src="#url#">hello</script>
<style type=
"#t#">a{color:red}</style>
</cfoutput>
<script <cfif x>type="text/plain"</cfif>>hello</script>
<script type=
"text/plain">hello</script>
<cfoutput>
<div>
<script type="#kind#">const s = `a
   b`;
     x();</script>
</div>
</cfoutput>
<div>
    <div>
<script type="text/plain">  
a
    b  
   </script>
    </div>
</div>
<cfoutput>
<script type="text/x-template">
  <div>
      <cfif x>#y#</cfif>
  </div>
</script>
</cfoutput>
<style type="text/less">
@c: red;
  .a { color: @c; }
</style>
