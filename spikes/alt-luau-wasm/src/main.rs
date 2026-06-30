use mlua::Lua;
fn main() {
    let lua = Lua::new();
    let v: i64 = lua.load("local x=0 for i=1,1000000 do x=x+i end return x").eval().unwrap();
    println!("sum={}", v);
}
