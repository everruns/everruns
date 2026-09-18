# Record the real example command. record.sh supplies credentials through Doppler.
Output examples/spam-triage/demo/.demo.tmp.gif
Set Shell "bash"
Set Width 1280
Set Height 900
Set FontSize 14
Set Theme "Catppuccin Mocha"
Set TypingSpeed 20ms
Set Framerate 12

Hide
Type "exec > >(tee examples/spam-triage/demo/.session.tmp) 2>&1"
Enter
Wait
Type "cargo build -q -p everruns-spam-triage"
Enter
Wait
Type "clear"
Enter

Show
Type@35ms "cargo run -q -p everruns-spam-triage -- --limit 30"
Sleep 700ms
Enter
Wait+Screen /screening/
Sleep 2s
Hide
Sleep 40s
Show
Wait+Screen /SUMMARY/
Sleep 12s
