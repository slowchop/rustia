local t = {
    value = 37,
    add = function(self, x)
        return self.value + x
    end,
}

return t:add(5)
