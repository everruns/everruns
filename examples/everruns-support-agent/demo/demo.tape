# Record the real example command. record.sh supplies credentials through Doppler.
Output examples/everruns-support-agent/demo/.demo.tmp.gif
Set Shell "bash"
Set Width 1280
Set Height 900
Set FontSize 18
Set Theme "Catppuccin Mocha"
Set TypingSpeed 20ms
Set Framerate 12

Hide
Type "exec > >(tee examples/everruns-support-agent/demo/.session.tmp) 2>&1"
Enter
Wait
Type "cargo build -q -p everruns-framework-support-agent"
Enter
Wait
Type "clear"
Enter

Show
Type "cargo run -q -p everruns-framework-support-agent -- --interactive"
Sleep 700ms
Enter
Wait+Screen /Question:/
Type@35ms "How do I resume a durable Framework session after restarting my process?"
Sleep 700ms
Enter
Sleep 2s
Wait+Line@120s />$/
Sleep 10s
