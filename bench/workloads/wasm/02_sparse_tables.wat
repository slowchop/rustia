(module
  (memory 1)
  (func $f (export "f") (param $n i64) (result i64)
    (local $iter i64)
    (local $i i64)
    (local $index i64)
    (local $checksum i64)

    (loop $outer
      local.get $iter
      local.get $n
      i64.lt_u
      if
        i64.const 0
        local.set $i

        (loop $fill
          local.get $i
          i64.const 128
          i64.lt_u
          if
            local.get $i
            i64.const 7919
            i64.mul
            local.get $iter
            i64.add
            i64.const 2047
            i64.and
            local.set $index

            local.get $index
            i32.wrap_i64
            i32.const 3
            i32.shl
            local.get $i
            local.get $iter
            i64.add
            i64.store

            local.get $i
            i64.const 1
            i64.add
            local.set $i
            br $fill
          end
        )

        i64.const 0
        local.set $i

        (loop $read
          local.get $i
          i64.const 128
          i64.lt_u
          if
            local.get $i
            i64.const 7919
            i64.mul
            local.get $iter
            i64.add
            i64.const 2047
            i64.and
            local.set $index

            local.get $checksum
            local.get $index
            i32.wrap_i64
            i32.const 3
            i32.shl
            i64.load
            i64.add
            i64.const 1000000007
            i64.rem_u
            local.set $checksum

            local.get $i
            i64.const 1
            i64.add
            local.set $i
            br $read
          end
        )

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
