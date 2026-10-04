# A live model run. record.sh supplies both provider keys through Doppler.
Output examples/foreman-agent/demo/.demo.tmp.gif
Set Shell "bash"
Set Width 1440
Set Height 900
Set FontSize 18
Set Theme "Catppuccin Mocha"
Set TypingSpeed 20ms
Set Framerate 12

Hide
Type "unset NO_COLOR; export PS1='> '; exec > >(tee examples/foreman-agent/demo/.session.tmp) 2>&1"
Enter
Wait
Type "clear"
Enter

Show
Type@35ms "cargo run -q -p everruns-foreman-agent --bin foreman -- demo --interactive"
Sleep 700ms
Enter
Wait+Line@30s /Job:$/
Sleep 2s
Type@30ms "Replace flat rates with these tiers, add boundary tests, update README, and run the suite."
Sleep 1s
Enter
Wait+Line@180s />$/
Sleep 7s
Hide
Type "printf '%s' $? > examples/foreman-agent/demo/.exit.tmp"
Enter
Wait
