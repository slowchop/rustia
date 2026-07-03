(module
  (func $f (export "f") (param $n i64) (result i64)
    (local $iter i64)
    (local $j i64)
    (local $state i64)
    (local $checksum i64)

    (loop $outer
      local.get $iter
      local.get $n
      i64.lt_u
      if
        local.get $iter
        local.set $state
        i64.const 0
        local.set $j

        (loop $step
          local.get $j
          i64.const 16
          i64.lt_u
          if
            local.get $state
            local.get $j
            i64.const 1664525
            i64.mul
            i64.add
            i64.const 1013904223
            i64.add
            i64.const 1000000007
            i64.rem_u
            local.set $state

            local.get $j
            i64.const 1
            i64.add
            local.set $j
            br $step
          end
        )

        local.get $checksum
        local.get $state
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
