(module
  (func $f (export "f") (param $n i64) (result i64)
    (local $iter i64)
    (local $x f64)
    (local $y f64)
    (local $checksum i64)

    (loop $outer
      local.get $iter
      local.get $n
      i64.lt_u
      if
        local.get $iter
        i64.const 97
        i64.rem_u
        f64.convert_i64_u
        f64.const 1
        f64.add
        f64.sqrt
        local.set $x

        local.get $iter
        i64.const 360
        i64.rem_u
        f64.convert_i64_u
        f64.const 0.017453292519943295
        f64.mul
        local.get $x
        f64.mul
        f64.abs
        f64.const 1000000
        f64.mul
        f64.floor
        local.set $y

        local.get $checksum
        local.get $y
        i64.trunc_f64_u
        i64.const 1000003
        i64.rem_u
        i64.add
        i64.const 1000000007
        i64.rem_u
        local.set $checksum

        local.get $iter
        i64.const 1
        i64.add
        local.set $iter
        br $outer
      end
    )

    local.get $checksum
  )
)
