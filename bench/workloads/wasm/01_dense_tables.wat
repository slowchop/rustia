(module
  (memory 1)
  (func $f (export "f") (param $n i64) (result i64)
    (local $iter i64)
    (local $i i64)
    (local $sum i64)
    (local $value i64)
    (local $checksum i64)

    (loop $outer
      local.get $iter
      local.get $n
      i64.lt_u
      if
        i64.const 0
        local.set $i
        i64.const 0
        local.set $sum

        (loop $fill
          local.get $i
          i64.const 256
          i64.lt_u
          if
            local.get $i
            i64.const 1103515245
            i64.mul
            local.get $iter
            i64.const 12345
            i64.mul
            i64.add
            i64.const 1000003
            i64.rem_u
            local.set $value

            local.get $i
            i32.wrap_i64
            i32.const 3
            i32.shl
            local.get $value
            i64.store

            local.get $sum
            local.get $value
            i64.add
            local.set $sum

            local.get $i
            i64.const 1
            i64.add
            local.set $i
            br $fill
          end
        )

        local.get $checksum
        local.get $sum
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
