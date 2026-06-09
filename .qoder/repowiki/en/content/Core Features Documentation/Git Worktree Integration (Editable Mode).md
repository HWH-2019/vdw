# Git Worktree Integration (Editable Mode)

<cite>
**Referenced Files in This Document**
- [manager.rs](file://vdw/src/worktree/manager.rs)
- [git_ops.rs](file://vdw/src/worktree/git_ops.rs)
- [worktree.rs](file://vdw/src/commands/worktree.rs)
- [mod.rs](file://vdw/src/worktree/mod.rs)
- [workspace.rs](file://vdw/src/types/workspace.rs)
- [object_pool.rs](file://vdw/src/store/object_pool.rs)
- [alternates.rs](file://vdw/src/store/alternates.rs)
- [engine.rs](file://vdw/src/link/engine.rs)
- [main.rs](file://vdw/src/main.rs)
- [worktree_integration_test.rs](file://vdw/tests/worktree_integration_test.rs)
- [link.rs](file://vdw/src/commands/link.rs)
- [manager.rs](file://vdw/src/store/manager.rs)
- [workspace.rs](file://vdw/src/types/workspace.rs)
- [editable_locks.rs](file://vdw/src/store/editable_locks.rs)
- [config.rs](file://vdw/src/types/config.rs)
- [editableLockService.ts](file://vdw-vscode/src/services/editableLockService.ts)
</cite>

## Update Summary
**Changes Made**
- Enhanced multi-workspace shared editing capabilities with new editable lock system for coordinating access across multiple workspaces
- Implemented global editable lock registry with file-based concurrency control using exclusive file locks
- Added cross-platform conflict detection and resolution for shared worktree access
- Integrated lock management into link command workflow for automatic lock acquisition/release
- Added comprehensive stale lock cleanup mechanism to prevent orphaned worktree locks
- Enhanced VS Code extension with client-side lock checking for real-time conflict detection

## Table of Contents
1. [Introduction](#introduction)
2. [Project Structure](#project-structure)
3. [Core Components](#core-components)
4. [Architecture Overview](#architecture-overview)
5. [Detailed Component Analysis](#detailed-component-analysis)
6. [Enhanced Multi-Workspace Shared Editing](#enhanced-multi-workspace-shared-editing)
7. [Editable Lock System Implementation](#editable-lock-system-implementation)
8. [Cross-Platform Conflict Resolution](#cross-platform-conflict-resolution)
9. [Client-Side Integration](#client-side-integration)
10. [Enhanced Backend Repository Management](#enhanced-backend-repository-management)
11. [Improved Worktree Creation Logic](#improved-worktree-creation-logic)
12. [Automatic HEAD Fixing Mechanisms](#automatic-head-fixing-mechanisms)
13. [Dependency Analysis](#dependency-analysis)
14. [Performance Considerations](#performance-considerations)
15. [Troubleshooting Guide](#troubleshooting-guide)
16. [Conclusion](#conclusion)
17. [Appendices](#appendices)

## Introduction
This document explains the comprehensive Git worktree integration feature for editable mode in the virtual development workspace. The implementation provides independent working copies under a shared Git object library, with robust bare repository management, symlink-based logical path mapping, and full compatibility with standard Git operations. The system includes detailed lifecycle management, cleanup procedures, and seamless IDE integration through the link engine.

**Updated** The implementation now features an enhanced multi-workspace shared editing capabilities with a new editable lock system that coordinates access across multiple workspaces while maintaining data integrity and preventing conflicts. The system includes global lock registry management, cross-platform conflict detection, and comprehensive stale lock cleanup mechanisms.

## Project Structure
The worktree integration is organized into several key modules within the VDW codebase:

```mermaid
graph TB
subgraph "Worktree Module"
A["worktree/mod.rs<br/>Public API re-exports"]
B["worktree/manager.rs<br/>Core worktree operations"]
C["worktree/git_ops.rs<br/>Git repository utilities"]
end
subgraph "Command Layer"
D["commands/worktree.rs<br/>CLI interface"]
E["main.rs<br/>Command routing"]
F["commands/link.rs<br/>Link command with worktree logic"]
end
subgraph "Integration Modules"
G["types/workspace.rs<br/>Workspace state types"]
H["store/object_pool.rs<br/>Enhanced repository management"]
I["store/alternates.rs<br/>Object sharing"]
J["link/engine.rs<br/>File linking"]
K["store/manager.rs<br/>Store management"]
L["store/editable_locks.rs<br/>Multi-workspace lock coordination"]
M["types/config.rs<br/>Lock registry data structures"]
end
subgraph "VS Code Extension"
N["editableLockService.ts<br/>Client-side lock checking"]
end
A --> B
A --> C
D --> B
D --> H
E --> D
F --> B
F --> H
F --> L
G --> J
H --> I
K --> H
L --> M
N --> L
```

**Diagram sources**
- [mod.rs:1-9](file://vdw/src/worktree/mod.rs#L1-L9)
- [manager.rs:1-474](file://vdw/src/worktree/manager.rs#L1-L474)
- [git_ops.rs:1-382](file://vdw/src/worktree/git_ops.rs#L1-L382)
- [worktree.rs:1-196](file://vdw/src/commands/worktree.rs#L1-L196)
- [link.rs:1-541](file://vdw/src/commands/link.rs#L1-L541)
- [editable_locks.rs:1-186](file://vdw/src/store/editable_locks.rs#L1-L186)
- [config.rs:373-396](file://vdw/src/types/config.rs#L373-L396)
- [editableLockService.ts:1-61](file://vdw-vscode/src/services/editableLockService.ts#L1-L61)

**Section sources**
- [mod.rs:1-9](file://vdw/src/worktree/mod.rs#L1-L9)
- [manager.rs:1-474](file://vdw/src/worktree/manager.rs#L1-L474)
- [git_ops.rs:1-382](file://vdw/src/worktree/git_ops.rs#L1-L382)
- [worktree.rs:1-196](file://vdw/src/commands/worktree.rs#L1-L196)
- [link.rs:1-541](file://vdw/src/commands/link.rs#L1-L541)
- [editable_locks.rs:1-186](file://vdw/src/store/editable_locks.rs#L1-L186)
- [config.rs:373-396](file://vdw/src/types/config.rs#L373-L396)
- [editableLockService.ts:1-61](file://vdw-vscode/src/services/editableLockService.ts#L1-L61)

## Core Components

### Worktree Manager
The worktree manager provides the core functionality for creating, listing, and removing Git worktrees with comprehensive error handling and fallback mechanisms:

- **Create Worktree**: Supports both git2 native API and git CLI fallback for maximum compatibility
- **List Worktrees**: Enumerates all active worktrees with detailed branch information  
- **Remove Worktrees**: Handles both unlocked and locked worktrees with force removal capability
- **Sanitization**: Converts branch names to safe directory names (slashes become hyphens)

**Updated** Enhanced with robust fallback mechanisms that automatically switch from git2 API to git CLI when native API fails, ensuring worktree operations succeed across different environments.

### Git Operations Utilities
Comprehensive Git repository management utilities for branch operations and repository manipulation:

- **Repository Management**: Open bare and regular repositories with proper error handling
- **Branch Operations**: List local and remote branches, find or create local branches
- **Commit Operations**: Get branch commits and HEAD commit information
- **Testing Utilities**: Helper functions for creating test repositories and commits

**Updated** Enhanced with improved branch resolution that handles both local and remote branch creation, with automatic fallback mechanisms for branch detection.

### CLI Interface
Command-line interface for worktree management with workspace integration:

- **List Command**: Shows all worktrees across dependencies with status information
- **Create Command**: Creates worktrees for specific repositories and branches
- **Delete Command**: Removes worktrees with automatic cleanup
- **Workspace Integration**: Integrates with VDW configuration and store management

**Updated** Complete CLI implementation with proper argument parsing, error handling, and integration with the enhanced object pool management.

### Editable Lock System
**New** Comprehensive lock management system for coordinating access across multiple workspaces:

- **Global Registry**: Centralized lock tracking across all workspaces and repositories
- **File-Based Concurrency**: Exclusive file locks prevent simultaneous modifications
- **Conflict Detection**: Real-time detection of conflicting worktree access attempts
- **Stale Lock Cleanup**: Automatic removal of orphaned locks when worktrees are deleted
- **Cross-Platform Support**: Consistent behavior across Windows, macOS, and Linux

**Section sources**
- [manager.rs:13-53](file://vdw/src/worktree/manager.rs#L13-L53)
- [git_ops.rs:6-16](file://vdw/src/worktree/git_ops.rs#L6-L16)
- [worktree.rs:8-73](file://vdw/src/commands/worktree.rs#L8-L73)
- [editable_locks.rs:17-34](file://vdw/src/store/editable_locks.rs#L17-L34)
- [editable_locks.rs:67-106](file://vdw/src/store/editable_locks.rs#L67-L106)

## Architecture Overview
The worktree integration follows a layered architecture with clear separation of concerns:

```mermaid
graph TB
subgraph "CLI Layer"
CLI["commands/worktree.rs<br/>User interface"]
CMD["main.rs<br/>Command routing"]
LINK["commands/link.rs<br/>Link command with worktree logic"]
end
subgraph "Manager Layer"
WM["worktree/manager.rs<br/>Worktree operations"]
GO["worktree/git_ops.rs<br/>Git utilities"]
END
subgraph "Store Integration"
OP["store/object_pool.rs<br/>Enhanced repository management"]
ALT["store/alternates.rs<br/>Object sharing"]
SM["store/manager.rs<br/>Store management"]
EL["store/editable_locks.rs<br/>Multi-workspace lock coordination"]
END
subgraph "Workspace Layer"
WS["types/workspace.rs<br/>State management"]
LE["link/engine.rs<br/>File linking"]
CFG["types/config.rs<br/>Lock registry structures"]
END
subgraph "VS Code Extension"
VSC["editableLockService.ts<br/>Client-side lock checking"]
END
CLI --> WM
CMD --> CLI
LINK --> WM
LINK --> EL
WM --> GO
WM --> OP
OP --> ALT
OP --> SM
EL --> CFG
WM --> WS
WS --> LE
VSC --> EL
```

**Diagram sources**
- [worktree.rs:1-196](file://vdw/src/commands/worktree.rs#L1-L196)
- [manager.rs:1-474](file://vdw/src/worktree/manager.rs#L1-L474)
- [git_ops.rs:1-382](file://vdw/src/worktree/git_ops.rs#L1-L382)
- [link.rs:1-541](file://vdw/src/commands/link.rs#L1-L541)
- [object_pool.rs:18-75](file://vdw/src/store/object_pool.rs#L18-L75)
- [editable_locks.rs:1-186](file://vdw/src/store/editable_locks.rs#L1-L186)
- [config.rs:373-396](file://vdw/src/types/config.rs#L373-L396)
- [editableLockService.ts:1-61](file://vdw-vscode/src/services/editableLockService.ts#L1-L61)

## Detailed Component Analysis

### Worktree Lifecycle Management
The worktree lifecycle is fully automated with comprehensive error handling and cleanup:

```mermaid
sequenceDiagram
participant User as "User"
participant CLI as "vdw CLI"
participant Manager as "Worktree Manager"
participant GitOps as "Git Operations"
participant Store as "Store Manager"
participant Lock as "Editable Lock System"
User->>CLI : vdw worktree create <repo> <branch>
CLI->>Store : Find bare repo path
Store-->>CLI : Bare repo path
CLI->>Lock : check_editable_lock(repo, branch, workspace)
Lock-->>CLI : Conflict status
CLI->>Manager : create_worktree(bare_path, branch, base_path)
Manager->>GitOps : open_bare_repo(bare_path)
GitOps-->>Manager : Repository handle
Manager->>GitOps : find_or_create_local_branch(branch)
GitOps-->>Manager : Branch reference
Manager->>Manager : create_worktree_via_git2()
Manager-->>CLI : Worktree path
CLI->>Lock : acquire_editable_lock(repo, branch, workspace, worktree)
Lock-->>CLI : Lock acquired
CLI-->>User : Success message
```

**Diagram sources**
- [worktree.rs:75-111](file://vdw/src/commands/worktree.rs#L75-L111)
- [manager.rs:13-53](file://vdw/src/worktree/manager.rs#L13-L53)
- [manager.rs:101-120](file://vdw/src/worktree/manager.rs#L101-L120)
- [editable_locks.rs:67-106](file://vdw/src/store/editable_locks.rs#L67-L106)
- [editable_locks.rs:108-139](file://vdw/src/store/editable_locks.rs#L108-L139)

### Editable Mode Implementation
The editable mode provides isolated working copies while sharing Git objects:

- **Isolation**: Each worktree maintains independent HEAD, index, and working tree
- **Sharing**: All worktrees share the same object database through alternates
- **Initialization**: Automatic .git directory setup for editable dependencies
- **Cleanup**: Proper removal of worktrees with directory cleanup

```mermaid
flowchart TD
Start(["Editable Dependency Detected"]) --> CheckStore["Check Store for Bare Repo"]
CheckStore --> |Exists| CheckLock["Check Global Editable Lock"]
CheckStore --> |Missing| CloneRepo["Clone to Store with Alternates"]
CheckLock --> |Conflict| HandleConflict["Handle Lock Conflict"]
CheckLock --> |Available| CreateWT["Create Worktree"]
CloneRepo --> CreateWT
CreateWT --> SetupGit["Setup .git Directory"]
SetupGit --> AcquireLock["Acquire Global Lock"]
AcquireLock --> CreateLinks["Create Symlink Links"]
CreateLinks --> IDEOpen["IDE Opens Worktree"]
IDEOpen --> EditMode["Full Git Operations Available"]
EditMode --> ReleaseLock["Release Lock on Unlink"]
ReleaseLock --> End(["Development Complete"])
```

**Diagram sources**
- [manager.rs:13-53](file://vdw/src/worktree/manager.rs#L13-L53)
- [object_pool.rs:18-75](file://vdw/src/store/object_pool.rs#L18-L75)
- [alternates.rs:6-26](file://vdw/src/store/alternates.rs#L6-L26)
- [editable_locks.rs:67-106](file://vdw/src/store/editable_locks.rs#L67-L106)
- [editable_locks.rs:108-139](file://vdw/src/store/editable_locks.rs#L108-L139)

**Section sources**
- [manager.rs:13-53](file://vdw/src/worktree/manager.rs#L13-L53)
- [object_pool.rs:18-75](file://vdw/src/store/object_pool.rs#L18-L75)
- [alternates.rs:6-26](file://vdw/src/store/alternates.rs#L6-L26)
- [editable_locks.rs:67-106](file://vdw/src/store/editable_locks.rs#L67-L106)
- [editable_locks.rs:108-139](file://vdw/src/store/editable_locks.rs#L108-L139)

### Bare Repository Management
The system manages bare repositories with object sharing capabilities:

- **Object Pool**: Centralized storage of Git objects across multiple repositories
- **Alternates Configuration**: Uses Git alternates to reference shared object databases
- **Reference Counting**: Tracks repository usage for cleanup management
- **Metadata Management**: Stores repository information and access timestamps

```mermaid
graph TB
subgraph "Store Layout"
SP["~/.vdw/store/"]
OBJ["objects/"]
RP["repo-name-<hash>/"]
BG["bare.git<br/>Bare repository"]
ALT["objects/info/alternates<br/>Object sharing"]
MET["metadata.json<br/>Repository info"]
END
SP --> OBJ
OBJ --> RP
RP --> BG
RP --> ALT
RP --> MET
ALT --> |"References"| OBJ
```

**Diagram sources**
- [object_pool.rs:20-75](file://vdw/src/store/object_pool.rs#L20-L75)
- [alternates.rs:6-26](file://vdw/src/store/alternates.rs#L6-L26)

**Section sources**
- [object_pool.rs:20-75](file://vdw/src/store/object_pool.rs#L20-L75)
- [alternates.rs:6-26](file://vdw/src/store/alternates.rs#L6-L26)

### Symlink-Based Logical Path Mapping
The link engine creates seamless integration between worktrees and the logical workspace:

- **Platform Detection**: Automatically selects optimal linking strategy (Hard Link, Symlink, Junction)
- **Scope Filtering**: Applies include/exclude patterns for selective linking
- **Cross-Platform Support**: Windows junction points, Unix hard links/symlinks
- **Cleanup Support**: Proper removal of links with directory cleanup

```mermaid
graph LR
P["Project Workspace"]
S["Symlinked Paths"]
W["Worktree Directories"]
L["Link Engine"]
E["Engine Strategy"]
P --> S
S --> W
L --> E
E --> |"Hard Link"| L
E --> |"Symlink"| L
E --> |"Junction"| L
```

**Diagram sources**
- [engine.rs:9-65](file://vdw/src/link/engine.rs#L9-L65)
- [engine.rs:67-81](file://vdw/src/link/engine.rs#L67-L81)

**Section sources**
- [engine.rs:9-65](file://vdw/src/link/engine.rs#L9-L65)
- [engine.rs:67-81](file://vdw/src/link/engine.rs#L67-L81)

## Enhanced Multi-Workspace Shared Editing

### Global Editable Lock Registry
**New** The system implements a comprehensive global lock registry that coordinates access across multiple workspaces:

- **Centralized Tracking**: All worktree locks are stored in a single JSON file in the VDW home directory
- **Exclusive File Locking**: Uses filesystem-level exclusive locks to prevent concurrent modifications
- **Cross-Process Safety**: Ensures data integrity when multiple processes or workspaces access the same repository
- **Version Management**: Maintains registry versioning for backward compatibility

**Updated** The lock registry structure includes comprehensive metadata tracking including repository ID, branch information, workspace path, worktree path, and timestamp for each lock entry.

**Section sources**
- [editable_locks.rs:12-15](file://vdw/src/store/editable_locks.rs#L12-L15)
- [editable_locks.rs:18-34](file://vdw/src/store/editable_locks.rs#L18-L34)
- [config.rs:373-396](file://vdw/src/types/config.rs#L373-L396)

## Editable Lock System Implementation

### Lock Acquisition and Management
**New** The editable lock system provides sophisticated lock management for shared worktree access:

- **Conflict Detection**: Automatically checks for existing locks on the same repo+branch combination
- **Same Workspace Allowance**: Allows the same workspace to re-acquire locks (supports relinking)
- **Stale Lock Cleanup**: Automatically removes locks for worktrees that no longer exist
- **Graceful Degradation**: Continues operation even if lock file becomes corrupted

```mermaid
flowchart TD
Start(["Request Lock Acquisition"]) --> LoadRegistry["Load Lock Registry"]
LoadRegistry --> ScanConflicts["Scan for Conflicting Locks"]
ScanConflicts --> |Conflict Found| CheckValidity["Check Worktree Validity"]
ScanConflicts --> |No Conflict| CreateLock["Create New Lock Entry"]
CheckValidity --> |Valid| ReturnConflict["Return Conflict Information"]
CheckValidity --> |Invalid| CleanupStale["Cleanup Stale Lock"]
CleanupStale --> CreateLock
CreateLock --> SaveRegistry["Save Registry with Exclusive Lock"]
SaveRegistry --> Success["Lock Acquired Successfully"]
ReturnConflict --> End(["Operation Blocked"])
Success --> End(["Operation Proceeds"])
```

**Diagram sources**
- [editable_locks.rs:67-106](file://vdw/src/store/editable_locks.rs#L67-L106)
- [editable_locks.rs:108-139](file://vdw/src/store/editable_locks.rs#L108-L139)
- [editable_locks.rs:36-65](file://vdw/src/store/editable_locks.rs#L36-L65)

**Section sources**
- [editable_locks.rs:67-106](file://vdw/src/store/editable_locks.rs#L67-L106)
- [editable_locks.rs:108-139](file://vdw/src/store/editable_locks.rs#L108-L139)
- [editable_locks.rs:36-65](file://vdw/src/store/editable_locks.rs#L36-L65)

### Lock Release and Cleanup
**New** Comprehensive lock release and cleanup mechanisms ensure proper resource management:

- **Selective Release**: Only releases locks owned by the current workspace
- **Automatic Cleanup**: Periodic cleanup of stale locks where worktrees no longer exist
- **Atomic Operations**: File-based atomic operations using exclusive locks
- **Logging and Monitoring**: Detailed logging for debugging and monitoring lock operations

**Section sources**
- [editable_locks.rs:141-161](file://vdw/src/store/editable_locks.rs#L141-L161)
- [editable_locks.rs:163-185](file://vdw/src/store/editable_locks.rs#L163-L185)

## Cross-Platform Conflict Resolution

### Platform-Agnostic Lock Management
**New** The lock system provides consistent behavior across different operating systems:

- **File-Based Locking**: Uses filesystem-level locking mechanisms available on all platforms
- **Consistent API**: Abstracts platform differences behind a unified interface
- **Cross-Process Coordination**: Ensures locks work correctly across multiple processes
- **Error Handling**: Graceful handling of platform-specific locking limitations

**Updated** The system automatically handles platform-specific file locking mechanisms, ensuring reliable lock acquisition and release on Windows, macOS, and Linux systems.

**Section sources**
- [editable_locks.rs:52-54](file://vdw/src/store/editable_locks.rs#L52-L54)
- [editable_locks.rs:36-65](file://vdw/src/store/editable_locks.rs#L36-L65)

## Client-Side Integration

### VS Code Extension Lock Checking
**New** The VS Code extension provides real-time conflict detection and prevention:

- **Real-Time Checking**: Automatically checks for lock conflicts when opening workspaces
- **User-Friendly Alerts**: Provides clear notifications about conflicting worktree access
- **Graceful Degradation**: Continues operation even if lock file is inaccessible
- **Workspace-Aware**: Respects the current workspace context for conflict detection

```mermaid
sequenceDiagram
participant User as "Developer"
participant VSCode as "VS Code Extension"
participant LockService as "EditableLockService"
participant FileSystem as "File System"
User->>VSCode : Open Workspace
VSCode->>LockService : checkEditableLock(repoId, branch)
LockService->>FileSystem : Read ~/.vdw/editable_locks.json
FileSystem-->>LockService : Lock registry data
LockService->>LockService : Analyze conflicts
LockService-->>VSCode : Conflict status
VSCode-->>User : Show appropriate UI feedback
```

**Diagram sources**
- [editableLockService.ts:28-59](file://vdw-vscode/src/services/editableLockService.ts#L28-L59)

**Section sources**
- [editableLockService.ts:28-59](file://vdw-vscode/src/services/editableLockService.ts#L28-L59)

### Integration with Link Command Workflow
**New** The editable lock system integrates seamlessly with the existing link command workflow:

- **Automatic Lock Acquisition**: Locks are automatically acquired when creating or reusing worktrees
- **Lock Release on Unlink**: Locks are automatically released when dependencies are unlinked
- **Conflict Prevention**: Prevents multiple workspaces from accessing the same worktree simultaneously
- **State Persistence**: Lock information persists across process restarts and system reboots

**Section sources**
- [link.rs:113-132](file://vdw/src/commands/link.rs#L113-L132)
- [link.rs:488-517](file://vdw/src/commands/link.rs#L488-L517)

## Enhanced Backend Repository Management

### Improved Branch Handling in Object Pool
The object pool management has been significantly enhanced with improved branch handling and automatic HEAD fixing mechanisms:

- **Branch Resolution**: Enhanced logic for determining the correct branch to use when creating worktrees
- **HEAD Fixing**: Automatic correction of bare repository HEAD references to prevent worktree creation failures
- **Fallback Mechanisms**: Robust fallback from git2 API to git CLI for maximum compatibility
- **Metadata Management**: Improved tracking of repository branches and commit information

**Updated** The `fix_bare_repo_head` function now intelligently detects the correct default branch (main/master/HEAD) and ensures the HEAD reference points to an existing branch, preventing worktree creation failures.

**Section sources**
- [object_pool.rs:152-204](file://vdw/src/store/object_pool.rs#L152-L204)
- [object_pool.rs:124-150](file://vdw/src/store/object_pool.rs#L124-L150)

## Improved Worktree Creation Logic

### Enhanced Link Command Integration
The link command now includes sophisticated worktree creation logic with intelligent branch resolution:

- **Existing Worktree Detection**: Scans the store for existing worktrees and reuses them when available
- **Conventional Path Scanning**: Checks conventional worktree locations in the store
- **Intelligent Branch Selection**: Determines the appropriate branch based on dependency configuration
- **Path Management**: Properly manages dep.local and dep.origin fields for editable dependencies

**Updated** The link command now includes comprehensive logic for detecting existing worktrees, scanning conventional paths, and intelligently managing worktree paths to ensure consistent behavior across re-link operations. The system now integrates lock management to coordinate access across multiple workspaces.

**Section sources**
- [link.rs:72-130](file://vdw/src/commands/link.rs#L72-L130)
- [link.rs:95-108](file://vdw/src/commands/link.rs#L95-L108)
- [link.rs:113-132](file://vdw/src/commands/link.rs#L113-L132)

## Automatic HEAD Fixing Mechanisms

### Comprehensive Bare Repository Management
The system now includes automatic HEAD fixing mechanisms to ensure bare repositories are properly configured:

- **Default Branch Detection**: Automatically detects whether the repository uses main or master as default branch
- **HEAD Reference Correction**: Ensures HEAD points to an existing branch reference
- **Local Branch Creation**: Creates local branch references from remote tracking branches when needed
- **Error Prevention**: Prevents worktree creation failures caused by invalid HEAD references

**Updated** The `fix_bare_repo_head` function provides comprehensive HEAD fixing that handles various repository configurations and ensures worktree operations succeed regardless of the repository's default branch naming convention.

**Section sources**
- [object_pool.rs:152-204](file://vdw/src/store/object_pool.rs#L152-L204)

## Dependency Analysis
The worktree integration has well-defined dependencies and clear separation of concerns:

```mermaid
graph LR
WM["worktree/manager.rs"] --> GO["worktree/git_ops.rs"]
WM --> TI["types/workspace.rs"]
WM --> OP["store/object_pool.rs"]
EL["store/editable_locks.rs"] --> CFG["types/config.rs"]
EL --> SM["store/manager.rs"]
CLI["commands/worktree.rs"] --> WM
CLI --> OP
CLI --> EL
LINK["commands/link.rs"] --> WM
LINK --> OP
LINK --> EL
MAIN["main.rs"] --> CLI
MAIN --> LINK
WS["types/workspace.rs"] --> LE["link/engine.rs"]
OP --> ALT["store/alternates.rs"]
SM --> OP
VSC["editableLockService.ts"] --> EL
```

**Diagram sources**
- [manager.rs:1-8](file://vdw/src/worktree/manager.rs#L1-L8)
- [worktree.rs:4-6](file://vdw/src/commands/worktree.rs#L4-L6)
- [link.rs:1-9](file://vdw/src/commands/link.rs#L1-L9)
- [main.rs:14-39](file://vdw/src/main.rs#L14-L39)
- [editable_locks.rs:1-186](file://vdw/src/store/editable_locks.rs#L1-L186)
- [config.rs:373-396](file://vdw/src/types/config.rs#L373-L396)
- [editableLockService.ts:1-61](file://vdw-vscode/src/services/editableLockService.ts#L1-L61)

**Section sources**
- [manager.rs:1-8](file://vdw/src/worktree/manager.rs#L1-L8)
- [worktree.rs:4-6](file://vdw/src/commands/worktree.rs#L4-L6)
- [link.rs:1-9](file://vdw/src/commands/link.rs#L1-L9)
- [main.rs:14-39](file://vdw/src/main.rs#L14-L39)
- [editable_locks.rs:1-186](file://vdw/src/store/editable_locks.rs#L1-L186)
- [config.rs:373-396](file://vdw/src/types/config.rs#L373-L396)
- [editableLockService.ts:1-61](file://vdw-vscode/src/services/editableLockService.ts#L1-L61)

## Performance Considerations
The worktree integration is designed for optimal performance and resource utilization:

- **Memory Efficiency**: Worktrees share object databases through alternates, reducing memory footprint
- **Disk Space Optimization**: Object deduplication across multiple worktrees and repositories
- **Network Efficiency**: Reference-based cloning minimizes bandwidth usage
- **Parallel Operations**: Asynchronous processing for multiple repository operations
- **Lazy Initialization**: Worktrees created only when needed for editable dependencies
- **Fallback Optimization**: Intelligent switching between git2 API and git CLI based on environment compatibility
- **Lock Contention Minimization**: Efficient lock acquisition and release mechanisms reduce blocking
- **Stale Lock Cleanup**: Automatic cleanup prevents accumulation of invalid lock entries

**Updated** Enhanced performance through robust fallback mechanisms that ensure worktree operations succeed even when git2 API is unavailable, with automatic detection of optimal operation modes. The new lock system minimizes contention through efficient file-based locking and automatic stale lock cleanup.

## Troubleshooting Guide

### Common Issues and Solutions

**Worktree Creation Failures**
- **Issue**: `git2 worktree API failed` - falls back to git CLI
- **Solution**: Ensure git CLI is available and properly configured
- **Prevention**: Check git installation and permissions

**Worktree Removal Problems**
- **Issue**: Locked worktree prevents removal
- **Solution**: Use `--force` flag or unlock the worktree first
- **Prevention**: Always close IDE/editor before deletion

**Branch Resolution Errors**
- **Issue**: Branch not found in local or remote
- **Solution**: Verify branch name and remote connectivity
- **Prevention**: Use `git branch -a` to list available branches

**Store Path Issues**
- **Issue**: Store not found or inaccessible
- **Solution**: Run `vdw init` to initialize the store
- **Prevention**: Ensure proper permissions on `~/.vdw/` directory

**HEAD Reference Issues**
- **Issue**: Invalid HEAD reference prevents worktree creation
- **Solution**: Automatic HEAD fixing resolves the issue
- **Prevention**: System automatically detects and fixes HEAD references

**Multi-Workspace Conflicts**
- **Issue**: Editable lock prevents worktree access
- **Solution**: Close conflicting workspace or wait for lock release
- **Prevention**: Coordinate with team members sharing the same repository

**Lock File Corruption**
- **Issue**: Corrupted editable_locks.json file
- **Solution**: System automatically rebuilds corrupted lock registry
- **Prevention**: Avoid manual modification of lock files

**Updated** Comprehensive error handling with detailed logging and fallback mechanisms for all failure scenarios, including automatic HEAD fixing for bare repositories and sophisticated multi-workspace conflict resolution through the editable lock system.

**Section sources**
- [manager.rs:40-49](file://vdw/src/worktree/manager.rs#L40-L49)
- [manager.rs:133-137](file://vdw/src/worktree/manager.rs#L133-L137)
- [worktree.rs:137-163](file://vdw/src/commands/worktree.rs#L137-L163)
- [editable_locks.rs:27-33](file://vdw/src/store/editable_locks.rs#L27-L33)
- [editable_locks.rs:53-54](file://vdw/src/store/editable_locks.rs#L53-L54)

## Conclusion
The comprehensive Git worktree integration provides a robust foundation for editable development workflows. The implementation offers:

- **Complete Lifecycle Management**: Full support for worktree creation, listing, and deletion
- **Robust Error Handling**: Graceful fallbacks and comprehensive error reporting
- **Performance Optimization**: Efficient object sharing and resource utilization
- **Seamless Integration**: Smooth IDE support through symlink-based mapping
- **Cross-Platform Compatibility**: Native support for Windows, macOS, and Linux
- **Enhanced Repository Management**: Automatic HEAD fixing and improved branch handling
- **Intelligent Worktree Logic**: Sophisticated worktree detection and reuse mechanisms
- **Multi-Workspace Coordination**: Advanced lock management system for shared editing
- **Conflict Prevention**: Real-time detection and resolution of workspace conflicts
- **Data Integrity**: Comprehensive protection against concurrent modifications

**Updated** The implementation is now fully tested and validated through comprehensive integration tests that verify all worktree operations including creation, listing, removal, and validation of worktree integrity. The enhanced backend repository management with automatic HEAD fixing mechanisms ensures reliable worktree operations across diverse environments. The new editable lock system provides sophisticated multi-workspace coordination with automatic conflict detection and resolution, making it an essential component of the VDW ecosystem.

The system enables developers to work on multiple repositories simultaneously while maintaining isolation and performance, with advanced coordination mechanisms that prevent conflicts and ensure data integrity across multiple workspaces and teams.

## Appendices

### Command Reference
- `vdw worktree list`: List all worktrees across dependencies
- `vdw worktree create <repo> <branch>`: Create worktree for specific repository and branch
- `vdw worktree delete <repo> <branch>`: Remove worktree with cleanup

**Updated** Complete CLI support for all worktree management operations with enhanced error handling, fallback mechanisms, and integrated lock management for multi-workspace coordination.

### Configuration Options
- Editable mode: Enable worktree creation for specific dependencies
- Branch specification: Target specific branches for worktree creation
- Store management: Automatic initialization and cleanup of worktree storage
- HEAD fixing: Automatic correction of bare repository HEAD references
- Lock management: Automatic coordination of worktree access across workspaces

### Best Practices
- Use descriptive branch names for worktrees
- Regular cleanup of unused worktrees
- Monitor disk usage with multiple worktrees
- Leverage alternates for efficient object sharing
- Maintain proper backup strategies for editable workspaces
- Ensure git CLI availability for maximum compatibility
- Coordinate with team members for shared repository access
- Monitor lock file health and clean up stale locks regularly
- Use consistent workspace naming conventions across team

**Updated** Enhanced with comprehensive testing coverage ensuring reliability and stability of worktree operations, including automatic HEAD fixing, intelligent worktree reuse mechanisms, and sophisticated multi-workspace coordination through the editable lock system.

**Section sources**
- [main.rs:76-94](file://vdw/src/main.rs#L76-L94)
- [worktree.rs:188-196](file://vdw/src/commands/worktree.rs#L188-L196)
- [link.rs:110-130](file://vdw/src/commands/link.rs#L110-L130)
- [editable_locks.rs:163-185](file://vdw/src/store/editable_locks.rs#L163-L185)