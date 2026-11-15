# Record the real interactive example. record.sh supplies credentials through Doppler.
Output examples/code-mode-mcp-agent/demo/.demo.tmp.gif
Set Shell "bash"
Set Width 1400
Set Height 960
Set FontSize 17
Set Theme "Catppuccin Mocha"
Set TypingSpeed 20ms
Set Framerate 12

Hide
Type "exec > >(tee examples/code-mode-mcp-agent/demo/.session.tmp) 2>&1"
Enter
Wait
Type "cargo build -q -p everruns-code-mode-mcp-agent"
Enter
Wait
Type "clear"
Enter

Show
Type "cargo run -q -p everruns-code-mode-mcp-agent -- --interactive"
Sleep 700ms
Enter
Wait+Screen /Question:/
Sleep 1s
Type@35ms "Which EU customers have overdue invoices? Give the three with the largest total overdue amount, in dollars."
Sleep 700ms
Enter
Sleep 2s
Wait+Line@240s />$/
Sleep 12s
