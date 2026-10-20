<cfoutput>
<a href="#url#">#doStuff('something', 'anothervalue', 'thirdvalue', 'fourth_thingy', 'fiftharg', 'sixth_whatever_this_is_really_long_to_handle')#</a>
<a href="#url#">#doStuff('something', 'anothervalue', 'thirdvalue', 'fourth_thingy', 'fiftharg', 'sixth_whatever_this_is_really_long_to_handle')# </a>
<a href="#url#"> #doStuff('something', 'anothervalue', 'thirdvalue', 'fourth_thingy', 'fiftharg', 'sixth_whatever_this_is_really_long_to_handle')#</a>
<a href="#url#"> #doStuff('something', 'anothervalue', 'thirdvalue', 'fourth_thingy', 'fiftharg', 'sixth_whatever_this_is_really_long_to_handle')# </a>
<a href="#url#">
#doStuff('something', 'anothervalue', 'thirdvalue', 'fourth_thingy', 'fiftharg', 'sixth_whatever_this_is_really_long_to_handle')#
</a>
<span>Total: #sum('something', 'anothervalue', 'thirdvalue', 'fourth_thingy', 'fiftharg', 'sixth_whatever_this_is_really_long_to_handle')#</span>
<td>Total: #sum('something', 'anothervalue', 'thirdvalue', 'fourth_thingy', 'fiftharg', 'sixth_whatever_this_is_really_long_to_handle')#</td>
<cfif x>#doStuff('something', 'anothervalue', 'thirdvalue', 'fourth_thingy', 'fiftharg', 'sixth_whatever_this_is_really_long_to_handle')#</cfif>
text<cfif x>#doStuff('something', 'anothervalue', 'thirdvalue', 'fourth_thingy', 'fiftharg', 'sixth_whatever_this_is_really_long_to_handle')#</cfif>more
<b><a href="#url#">#doStuff('something', 'anothervalue', 'thirdvalue', 'fourth_thingy', 'fiftharg', 'sixth_whatever_this_is_really_long_to_handle')#</a></b>
<a href="#url#">#doStuff('something',
'anothervalue', 'thirdvalue', 'fourth_thingy', 'fiftharg', 'sixth_whatever_this_is_really_long_to_handle')#</a>
<a href="#url#">text
more text</a>
<span>#first & " " & second & " " & third & " " & fourth & " " & fifth & " " & sixth & " " & seventh & " " & eighth & " " & ninth#</span>
<a href="#url#">#isDefined("url.something_long_enough") ? url.something_long_enough_value : variables.default_fallback_value_name_which_is_longer#</a>
</cfoutput>
