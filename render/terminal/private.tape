Type "feed > /dev/null"
Enter
Wait@30s /^\$\s*$/
Type "stat -c '%A  %n' ~/.local/state/pito-sync-feed{,/feed.db}"
Enter
Wait@10s /^\$\s*$/
Sleep 1500ms
Type "chmod 755 ~/.local/state/pito-sync-feed && chmod 644 ~/.local/state/pito-sync-feed/feed.db"
Enter
Wait@10s /^\$\s*$/
Type "stat -c '%A  %n' ~/.local/state/pito-sync-feed{,/feed.db}"
Enter
Wait@10s /^\$\s*$/
Sleep 1500ms
Type "feed > /dev/null"
Enter
Wait@30s /^\$\s*$/
Type "stat -c '%A  %n' ~/.local/state/pito-sync-feed{,/feed.db}"
Enter
Wait@10s /^\$\s*$/
Sleep 3s
