<cfoutput>
  #doStuff('something', 'anothervalue', 'thirdvalue', 'fourth_thingy', 'fiftharg', 'sixth_whatever_this_is_really_long_to_handle')#
  <cfset x = doStuff('something', 'anothervalue', 'thirdvalue', 'fourth_thingy', 'fiftharg', 'sixth_whatever_this_is_really_long_to_handle')>
  #doStuff('a', 'b')# stays on its line, and so do #now()# and #x#
  #x#more and text #x# text and #a##b# glued
  #doStuff('something', 'anothervalue', 'thirdvalue', 'fourth_thingy', 'fiftharg', 'sixth_whatever_this_is_really_long_to_handle')#more
  text #doStuff('something', 'anothervalue', 'thirdvalue', 'fourth_thingy', 'fiftharg', 'sixth_whatever_this_is_really_long_to_handle')# text
  #variables.service.doStuff('something', 'anothervalue', 'thirdvalue', 'fourth_thingy', 'fiftharg', 'sixth_whatever')#
  #first & " " & second & " " & third & " " & fourth & " " & fifth & " " & sixth & " " & seventh & " " & eighth & " " & ninth#
  #isDefined("url.something_long_enough") ? url.something_long_enough_value : variables.default_fallback_value_name_which_is_longer#
  #doStuff('something', // a comment
    'anothervalue')#
  #doStuff('a') // a comment ends the line, so the delimiters stay where they are
  #
</cfoutput>
<cfoutput>#doStuff('something', 'anothervalue', 'thirdvalue', 'fourth_thingy', 'fiftharg', 'sixth_whatever_this_is_really_long_to_handle')#</cfoutput>
<p><cfoutput>#doStuff('something', 'anothervalue', 'thirdvalue', 'fourth_thingy', 'fiftharg', 'sixth_whatever_this_is_really_long_to_handle')#</cfoutput></p>
<cfoutput>#a#</cfoutput>
