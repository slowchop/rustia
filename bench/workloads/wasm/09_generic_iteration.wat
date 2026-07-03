(module
  (func $f (export "f") (param $n i64) (result i64)
    (local $iter i64)
    (local $i i64)
    (local $checksum i64)

    (loop $outer
      local.get $iter
      local.get $n
      i64.lt_u
      if
        i64.const 0
        local.set $i

        (loop $array
          local.get $i
          i64.const 32
          i64.lt_u
          if
            local.get $checksum
            local.get $iter
            local.get $i
            i64.add
            i64.add
            i64.const 1000000007
            i64.rem_u
            local.set $checksum

            local.get $i
            i64.const 1
            i64.add
            local.set $i
            br $array
          end
        )

        i64.const 0
        local.set $i

        (loop $fields
          local.get $i
          i64.const 5
          i64.lt_u
          if
            local.get $checksum
            local.get $iter
            local.get $i
            i64.add
            i64.add
            i64.const 1000000007
            i64.rem_u
            local.set $checksum

            local.get $i
            i64.const 1
            i64.add
            local.set $i
            br $fields
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
