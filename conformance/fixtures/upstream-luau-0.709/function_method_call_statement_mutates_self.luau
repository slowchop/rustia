local t = {
    value = 0,
    set = function(self, x)
        self.value = x
    end,
}

t:set(12)
return t.value
