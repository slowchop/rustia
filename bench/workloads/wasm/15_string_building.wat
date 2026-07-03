(module
  (memory 1)

  (func $digits_len (param $x i64) (result i64)
    local.get $x
    i64.const 10
    i64.lt_u
    if (result i64)
      i64.const 1
    else
      i64.const 2
    end
  )

  (func $fragment_len (param $selector i64) (result i64)
    local.get $selector
    i64.const 4
    i64.rem_u
    i64.const 1
    i64.eq
    if (result i64)
      i64.const 4
    else
      i64.const 5
    end
  )

  (func $f (export "f") (param $n i64) (result i64)
    (local $iter i64)
    (local $i i64)
    (local $checksum i64)
    (local $part_len i64)
    (local $written i64)
    (local $offset i32)

    (loop $outer
      local.get $iter
      local.get $n
      i64.lt_u
      if
        i32.const 0
        local.set $offset
        i64.const 0
        local.set $i

        (loop $parts
          local.get $i
          i64.const 64
          i64.lt_u
          if
            local.get $i
            local.get $iter
            i64.add
            call $fragment_len
            local.get $i
            i64.const 17
            i64.rem_u
            call $digits_len
            i64.add
            local.get $iter
            i64.const 31
            i64.rem_u
            call $digits_len
            i64.add
            i64.const 3
            i64.add
            local.set $part_len

            i64.const 0
            local.set $written
            (loop $write
              local.get $written
              local.get $part_len
              i64.lt_u
              if
                local.get $offset
                local.get $written
                i32.wrap_i64
                i32.add
                local.get $iter
                local.get $i
                i64.add
                local.get $written
                i64.add
                i32.wrap_i64
                i32.store8

                local.get $written
                i64.const 1
                i64.add
                local.set $written
                br $write
              end
            )

            local.get $offset
            local.get $part_len
            i32.wrap_i64
            i32.add
            local.set $offset

            local.get $i
            i64.const 1
            i64.add
            local.set $i
            br $parts
          end
        )

        local.get $checksum
        local.get $offset
        i64.extend_i32_u
        i64.add
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
