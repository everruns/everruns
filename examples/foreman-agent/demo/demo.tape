# Record a real run: a scripted worker, a real classifier watching it.
# record.sh supplies TYPESAFE_API_KEY through Doppler.
Output examples/foreman-agent/demo/.demo.tmp.gif
Set Shell "bash"
Set Width 1240
Set Height 860
Set FontSize 15
Set Theme "Catppuccin Mocha"
Set TypingSpeed 20ms
Set Framerate 12

Hide
Type "exec > >(tee examples/foreman-agent/demo/.session.tmp) 2>&1"
Enter
Wait
Type "cargo build -q -p everruns-foreman-agent --bin foreman"
Enter
Wait
Type "clear"
Enter

Show
Type@35ms "cargo run -q -p everruns-foreman-agent --bin foreman -- demo --live-foreman"
Sleep 700ms
Enter
Sleep 2s
Wait+Line@180s />$/
Sleep 6s
