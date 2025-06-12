# Task List Edit Functionality

The task list example supports editing with cursor inversion, field cycling, and priority selection.

## Key Bindings

### Normal Mode:
- `j/k` or arrows: Navigate between tasks
- `space`: Toggle task completion status
- `e`: Edit task title
- `E`: Edit task description  
- `p`: Edit task priority
- `n`: Create new task
- `d`: Delete selected task
- `q`: Quit

### Edit Mode:
- `Enter`: Save the edit
- `Esc`: Cancel edit without saving
- `Tab` or `↓`: Cycle to next field (Title → Description → Priority → Title)
- `↑`: Cycle to previous field (Priority → Description → Title → Priority)

#### For Title & Description:
- `←/→`: Move cursor left/right
- `Backspace`: Delete character before cursor
- Type any character to insert at cursor position

#### For Priority:
- `←/→`: Cycle through priority levels (Low ⟷ Medium ⟷ High)
- Cannot type text (controlled selection only)

## Priority System
Priority editing uses **controlled selection** with three levels:
- 🟢 **Low**: Low priority tasks
- 🟡 **Medium**: Medium priority tasks (default)
- 🔴 **High**: High priority tasks

Use `←/→` arrows to cycle through these levels when editing priority. No text input required!

## How to Test

1. Run the task list example:
   ```bash
   cargo run --example task_list_example -- --auto --user test-enhanced-edit
   ```

2. Create some tasks with `n`

3. Select a task and try the enhanced editing:
   - Press `e` to edit the title - see the **proper cursor inversion**
   - Press `E` to edit the description - notice the **underlined field**
   - Press `p` to edit the priority - use `←/→` to **cycle priorities**

4. Try field cycling:
   - Press `Tab` or `↓` to cycle forward through fields
   - Press `↑` to cycle backward through fields
   - Watch the help text change based on which field you're editing

5. Notice the enhanced visuals:
   - **Inverted cursor** that highlights the actual character
   - **Underlined fields** instead of background highlighting
   - **Dynamic help text** that adapts to the current field
   - **Priority hints** showing `[←/→ to change]`

## Enhanced Visual Features

- ✅ **Enhanced cursor display**: Bright red `│` cursor with yellow background highlighting
- ✅ **Field highlighting**: Active field shows with `▶` arrow and yellow label
- ✅ **Dimmed inactive fields**: Non-active fields are grayed out during editing
- ✅ **Field cycling**: Tab/Up/Down arrows to seamlessly switch between fields
- ✅ **Current field indicator**: Help text shows which field you're editing
- ✅ **Empty field display**: Shows "(empty)" for empty descriptions when editing
- ✅ **Proper validation**: Priority values with smart defaults
- ✅ **Real-time sync**: Changes sync across clients when online, queue when offline
- ✅ **Activity notifications**: Updates logged in activity panel

## Visual Editing Experience

When you enter edit mode:
1. **Active field** gets a yellow `▶` arrow and bright yellow background
2. **Cursor** appears as a red `│` line with precise positioning
3. **Other fields** are dimmed to show they're not active
4. **Help text** updates to show current field and available commands
5. **Tab/arrows** let you smoothly cycle between Title → Description → Priority

The editing experience is now much more intuitive with clear visual feedback about which field is active and where your cursor is positioned!