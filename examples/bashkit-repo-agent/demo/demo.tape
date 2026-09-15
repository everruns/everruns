# Record the real interactive example. record.sh supplies credentials through Doppler.
Output examples/bashkit-repo-agent/demo/.demo.tmp.gif
Set Shell "bash"
Set Width 1280
Set Height 900
Set FontSize 18
Set Theme "Catppuccin Mocha"
Set TypingSpeed 20ms
Set Framerate 12

Hide
Type "exec > >(tee examples/bashkit-repo-agent/demo/.session.tmp) 2>&1"
Enter
Wait
Type "cargo build -q -p everruns-bashkit-repo-agent"
Enter
Wait
Type "clear"
Enter

Show
Type "cargo run -q -p everruns-bashkit-repo-agent -- --interactive"
Sleep 700ms
Enter
Wait+Screen /Task:/
Type@35ms "Cut release 0.2.0 for today's date. Follow the repository release process."
Sleep 700ms
Enter
Sleep 2s
Wait+Line@180s />$/
Sleep 10s
