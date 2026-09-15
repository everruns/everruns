# Record the real example command. record.sh supplies credentials through Doppler.
Output examples/support-agent/demo/.demo.tmp.gif
Set Shell "bash"
Set Width 1280
Set Height 900
Set FontSize 18
Set Theme "Catppuccin Mocha"
Set TypingSpeed 20ms
Set Framerate 12

Hide
Type "exec > >(tee examples/support-agent/demo/.session.tmp) 2>&1"
Enter
Wait
Type "cargo build -q -p everruns-support-agent"
Enter
Wait
Type "clear"
Enter

Show
Type "cargo run -q -p everruns-support-agent -- --interactive"
Sleep 700ms
Enter
Wait+Screen /Question:/
Type@35ms "Customer cust_mfa reset their password but cannot sign in. What is the next safe step?"
Sleep 700ms
Enter
Sleep 2s
Hide
Sleep 45s
Show
Wait+Screen /ANSWER/
Sleep 10s
