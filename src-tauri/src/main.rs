// 防止 Windows 下打包成控制台应用时弹出黑色终端窗口
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    dsh_desktop_lib::run()
}