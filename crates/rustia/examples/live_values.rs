use rustia::{Profile, Result, Rustia, ValueType};

fn main() -> Result<()> {
    let mut rustia = Rustia::new(Profile::ReproducibleStandard);
    let root = rustia.run_value(
        r#"
local root = {}
local shared = { answer = 42 }
root.self = root
root.left = shared
root.right = shared
root.bytes = buffer.fromstring(string.char(1, 0, 254))
return root
"#,
    )?;

    let table = root
        .as_table(&rustia)?
        .ok_or_else(|| rustia::Error::Embed("script did not return a table".to_owned()))?;
    let self_value = table.get(&mut rustia, "self")?;
    let left = table.get(&mut rustia, "left")?;
    let right = table.get(&mut rustia, "right")?;
    let bytes = table.get(&mut rustia, "bytes")?;

    assert_eq!(root.kind(&rustia)?, ValueType::Table);
    assert!(root.raw_equal(&mut rustia, &self_value)?);
    assert!(left.raw_equal(&mut rustia, &right)?);
    assert_eq!(bytes.buffer_bytes(&mut rustia)?, Some(vec![1, 0, 254]));

    println!("live values preserved the cycle, shared table identity, and buffer bytes");
    Ok(())
}
