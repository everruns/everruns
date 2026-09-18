# Record the real example command. record.sh supplies credentials through Doppler.
Output examples/open-mic-agent/demo/.demo.tmp.gif
Set Shell "bash"
Set Width 1280
Set Height 900
Set FontSize 18
Set Theme "Catppuccin Mocha"
Set TypingSpeed 20ms
Set Framerate 12

Hide
Type "exec > >(tee examples/open-mic-agent/demo/.session.tmp) 2>&1"
Enter
Wait
Type "cargo build -q -p everruns-open-mic-agent"
Enter
Wait
Type "clear"
Enter

Show
Type "cargo run -q -p everruns-open-mic-agent -- --interactive"
Sleep 700ms
Enter
Wait+Screen /Submission:/
Type@35ms "Book submission mic_late into tonight's line-up. Measure the bit before you decide."
Sleep 700ms
Enter
Sleep 2s
Hide
Sleep 25s
Show
# The buffer scrolls past one screen, so wait for the prompt to come back
# rather than for a line that has already scrolled away.
Wait@3m
Sleep 10s
