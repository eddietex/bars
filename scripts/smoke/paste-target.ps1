# A text box that takes focus, waits for something to be pasted, then saves
# what it got to the file given as the first argument and closes.
param([string]$Out)
Add-Type -AssemblyName System.Windows.Forms
$form = New-Object System.Windows.Forms.Form
$form.Text = 'Paste target'
$form.TopMost = $true
$form.StartPosition = 'CenterScreen'
$form.Width = 640
$form.Height = 200
$box = New-Object System.Windows.Forms.TextBox
$box.Multiline = $true
$box.Dock = 'Fill'
$form.Controls.Add($box)
$form.Add_Shown({ $form.Activate(); $box.Focus() })
$timer = New-Object System.Windows.Forms.Timer
$timer.Interval = 10000
$timer.Add_Tick({ $timer.Stop(); [IO.File]::WriteAllText($Out, $box.Text); $form.Close() })
$timer.Start()
[void]$form.ShowDialog()
